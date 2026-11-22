# Events and triggers (design)

**Status: release A implemented and shipped in SDK 1.10.0 / player 0.10.0 (2026-11-15); release B steps B1 (second batch of events) and B2 (removal of subscribe()/onChange()) implemented on branches (SDK 1.11.0-rc.1), not merged; B3–B6 not started.**

**For the owner (release B, 2026-11-22).** Points of steps B1 and B2 that are not the implementing agent's to decide; the implementation note in §4.6 has the rest.

1. *The store's lost `settings.set` is not fixed by the SDK change.* The SDK now refuses every request after `runtime.stop`, but the lost write was sent *before* the runner read `runtime.stop`: the host had removed the plugin and dropped it unanswered (`host/handle.rs`), and the runner then rejected it with `disabled`. Closing that window needs one of: the store not writing from a timer near its idle stop (it holds nothing then, so the stop is correct), or the host answering a request of a plugin it is stopping instead of dropping it (a small new host duty). Recommended: the store's fix, in B3.
2. *The player cannot depend on 1.11.0-rc.N before B3.* The built-ins call `plugin.player.subscribe()` at activation; with the rc they would fail at activation, and they build against the player's one SDK dependency. The B1 host code needs no SDK change, so the player branch stays on SDK 1.10.1 and switches to the rc together with the built-ins' migration in B3. Until then the player's `host-agreement` test compares the wake events without the eight new names when the SDK is older than 1.11.0.
3. *Overlap with lane L2.* Removing the two members also removes their entries from the SDK's `capability-scan.json` (its completeness test requires it), a file L2 deletes; and the host's `CAPABILITIES_REMOVED` message still tells authors to call `plugin.player.subscribe()`, which L2's option A removes. Whichever lane merges second resolves the conflict.

Ruled by the coordinator (2026-11-22): 1, the store's fix in B3, no new host duty, the SDK's refusal stays; 2, agreed; 3, L2's SDK PR merges first, then L1 merges main and follows L2 (no `capability-scan.json`, no `CAPABILITIES_REMOVED` wording, the code `builtin-only`).

Written 2026-11-14 by the API maintainer from the user's rulings of that day. First release: SDK 1.10.0 / player 0.10.0. The execution plan is [events-and-triggers-plan.md](../process/events-and-triggers-plan.md). This file is the authority for the design; where an earlier draft differs (the drafts v1–v3 in the local folder `design/wake-design/`), this file wins.

No code was written, no player was run and nothing was measured for this document. No example in it was compiled. Every measured value is quoted with its source; every other figure is an estimate and is listed in [§8](#8-estimates-not-yet-measured).

**How to read it.** What a plugin author writes: [§3](#3-the-api) (the whole API on one page, then four examples, each with a line-by-line explanation and a table of what happens over time). The exact rules: [§4](#4-exact-semantics). Why it is this way and what was set aside: [§5](#5-decisions-reasons-and-alternatives-set-aside). Where the rulings and today's code do not fit, and what this document decided about it: [§6](#6-where-the-rulings-meet-todays-code). The facts of today's system: [§1](#1-background-the-facts-today). Words: [§10](#10-glossary).

**How sources are written.** `P:` is the player repository (line numbers as of origin/main `a566d6ec`, 0.9.0; the commits after it change records only), `S:` the SDK repository (origin/main `1c69f57`, 1.9.0), `G:` the plugins repository (origin/main `a459534`), `T:` the wake-event taxonomy (a local page of 110 candidate triggers, `design/wake-events/index.html`; it has no stable line numbers, so its section names are used).

## 1. Background: the facts today

### 1.1 Who is in the system

| Name | What it is | Source |
| --- | --- | --- |
| Host | The player's own process, written in Rust. The window, the menus and mpv are inside it | P:doc/architecture/supervisor.md:1-5 |
| Plugin | A piece of TypeScript running in a Node process. It exchanges JSON lines (requests, answers, events) with the host over a pipe | S:docs/reference/events.md:8 |
| SDK | The TypeScript library plugin authors use. It wraps the pipe's messages into objects such as `plugin.player` and `plugin.when` | S:src/plugin/types/playback.ts:930 |
| Built-in plugin | One of the player's own 16 plugins (`vestra.*`). They share one Node process, the shared runner | P:doc/architecture/supervisor.md:23,170 |
| Third-party plugin | The 8 in the store and whatever a user installs. One process each | P:doc/architecture/supervisor.md:5,25 |
| Supervisor | The part of the host that decides whether each plugin's process should exist right now | P:doc/architecture/supervisor.md:3 |
| Playback thread | The one host thread that owns mpv (the code calls it the playback worker) | P:src/host/src/playback/backend.rs:176-183 |
| Core thread | The host's core (`vestra-core`, since player 0.10.2; [core-thread.md](core-thread.md)). The supervisor, the command table and condition evaluation live here; before 0.10.2 they ran on the UI thread | P:doc/architecture/supervisor.md:12 |
| UI thread | The host's main thread: Slint's event loop and the interface; a client of the core | P:doc/architecture/core-thread.md |

### 1.2 Standby and wake

Standby means the plugin's process is stopped while its entry points (menu items, keys, settings pages) stay in the interface. A wake means the host starts the process again because one of them was used.

```text
Host     [ entry points from the recording stay: menus, keys, settings pages, wake-event subscriptions ]
                                                              | a menu click, a key, a wake event
                                                              v
Plugin   [ running ][ idle 30 s:       ][ standby: no process          ][ start      ][ running            ]
process    last       recording written,  variables and timers of the     34-167 ms     backlog delivered
           request    then stopped        process are gone                              in order
```

| No. | Fact | Source |
| --- | --- | --- |
| F1 | A plugin idle for 30 s goes to standby: the host asks the playback thread whether it still holds a resource, writes its recording, and sends the stop | P:doc/architecture/supervisor.md:76,124; P:src/host/src/supervisor/limits.rs:196 |
| F2 | Any request from the plugin restarts its idle time | P:src/host/src/host/handle.rs:38-40 |
| F3 | A recording is a file that lists the entry points the plugin registered: commands, key bindings, settings pages, drop targets, resolvers, wake-event subscriptions, a background sidebar tab. At the next player start the host replays it without starting a process | P:doc/architecture/supervisor.md:61; P:src/host/src/host/supervisor/checkpoint.rs:1-5,38-58 |
| F4 | A recording is valid only for the same installed instance, package digest and version, and player version. After a player or package upgrade it is made again | P:src/host/src/host/supervisor/checkpoint.rs:28-36 |
| F5 | 5 interactive triggers wake a plugin in standby: `commands.execute` (menu, key, button), `source.resolveRequest`, `drops.drop`, `ui.callback`, `palette.lead`; also "a settings page is shown" and "a sidebar tab is shown" | P:src/host/src/host/supervisor/mod.rs:80-87; P:doc/architecture/supervisor.md:67-72 |
| F6 | 5 events wake, the wake events: `packages.changed`, `bindings.changed`, `commands.keys`, `appearance.changed`, `resolvers.fileTypes`. Each follows a deliberate change by the user or a plugin, never playback | P:src/host/src/capabilities/events.rs:25-40 |
| F7 | No other event wakes a plugin. A plugin that listens to one of them is not put in standby (1.3) | P:doc/architecture/supervisor.md:74; P:src/host/src/capabilities/events.rs:83-95 |
| F8 | A backlog holds at most 64 triggers per plugin. Wake events of the same name merge (the newest wins); interactive triggers do not merge | P:src/host/src/supervisor/limits.rs:206; P:src/host/src/supervisor/trigger.rs:100-111 |
| F9 | During player start-up and for 1 s after it settles, wake events wake nothing (the mute). When the mute ends, a listener whose state really changed is woken once | P:doc/architecture/supervisor.md:74,137 |
| F10 | A start longer than 400 ms shows "Starting *name*…" | P:doc/architecture/supervisor.md:74,136; P:src/host/src/supervisor/limits.rs:201 |
| F11 | A plugin holding "something only its process can serve" (live) is not put in standby. A third-party plugin holding such a thing at its ready is classed Resident for that run; the class is decided again at the next ready | P:doc/architecture/supervisor.md:24,28,63; P:src/host/src/capabilities/mod.rs:94-104 |
| F12 | Built-ins are not counted in the process budget and share one process. Parking a built-in saves no process | P:doc/architecture/supervisor.md:23,170,185 |
| F13 | Third-party processes have a budget: `clamp(physMiB / 2 × 0.30 / 85, 3, 16)`. 85 is the MB the formula allows per process, not a measured plugin size | P:src/host/src/supervisor/limits.rs:252-254; P:doc/architecture/supervisor.md:185 |
| F14 | At quit all plugin processes share 1 s: owed recordings are written, every process gets its stop at once, the rest is killed at the deadline. A plugin in standby receives nothing | P:doc/architecture/supervisor.md:133,199 |
| F15 | A third-party plugin that crashed goes to standby until its next use; wake events during its pause are held for the start after the pause | P:doc/architecture/supervisor.md:160,164 |
| F16 | A plugin may have at most 320 unanswered requests; the host drains requests in time-budgeted batches so one plugin cannot stall the core | P:doc/architecture/supervisor.md:172-174 |

**Wake latency that was measured** (T:"Wake latency"). Built-ins in the shared runner: 51 real wakes in 38 host logs, from spawn to ready: minimum 34 ms, median 82 ms, p90 116 ms, maximum 167 ms (the log line comes from P:src/host/src/host/supervisor/mod.rs:1031). The figure excludes the plugin's own handling of the trigger. A third-party plugin's own process has no measured record; the taxonomy's estimate is about 30 ms of process creation, plus 78–103 ms of Node start (measured with `node -e 0`), plus loading the SDK and activating: 0.15–0.3 s in total. Measuring it is a preparation step of the plan.

### 1.3 Today's 11 events, and the calls around them

An event is one JSON line the host sends on its own: `{"event": name, "data": …}`, only to plugins that subscribed to that name (S:docs/reference/events.md:8).

| Event | Content | How often | Wakes a plugin in standby | After `subscribe()` only | SDK members that listen |
| --- | --- | --- | --- | --- | --- |
| `source.state` | The whole state of the current source: phase, position, rate, volume, tracks | Every 100 ms while playing; at once when the phase, pause, rate or volume changes | no | yes | `Player.onChange`, `Subtitles.onChange`, video filters |
| `playback.backend` | The name of every mpv event | Every mpv event | no | yes | `Player.on` only |
| `window.state` | The main window: fullscreen, maximized, video area, picture-in-picture | Every resize step, every menu and focus change | no | yes | `MainWindow.onChange`, the panel slots' `onChange` |
| `timeline.changed` | Every timeline mark and range | Whenever someone changes a mark, at most every 100 ms | no | yes | `Player.on` only |
| `video.upscaling.changed`, `video.motion.changed` | The state of AI upscaling and motion smoothing | When it changes | no | no | `VideoUpscaling.onChange`, `VideoMotion.onChange` |
| `packages.changed`, `bindings.changed`, `commands.keys` | The plugin list, key bindings, this plugin's key labels | When the user changes a setting | **yes** | no | `Packages.onChange`, `Bindings.onChange`, `Commands.keys` |
| `appearance.changed`, `resolvers.fileTypes` | Theme and language; the extensions resolvers claim | When the user changes a setting | **yes** | yes | `Player.on` only |

Sources: names P:src/host/src/capabilities/events.rs:12-24; frequencies P:src/host/src/capabilities/events.rs:29-31 and P:src/host/src/playback/backend.rs:2144-2147,2191-2195; the last three columns S:docs/reference/events.md:17-29; the timeline's 100 ms P:src/host/src/timeline.rs:17-18.

**The four calls a plugin has for the player's state today** (SDK 1.9.0):

| Call | What it does | Source |
| --- | --- | --- |
| `await plugin.player.read()` | Reads the state and the timeline once at start-up, so that `plugin.player.state` holds what the player has; from then on the SDK also reads again after the plugin's own actions. Returns nothing. A second call reads nothing | S:src/plugin/types/playback.ts:1082-1129 |
| `await plugin.player.refresh()` | Asks the player for the state now and returns the snapshot | S:src/plugin/types/playback.ts:1023-1038 |
| `plugin.player.subscribe()` | Turns event delivery on for this run: `player.state` follows outside changes, and the six events marked "after `subscribe()` only" start to reach their listeners. The SDK's own subscription for the state copy is a *mirror*: the host does not count it as a listener | S:src/plugin/types/playback.ts:1039-1080; P:src/host/src/capabilities/events.rs:80-82,147-148 |
| `plugin.player.onChange(fn)` | `fn` runs with the new snapshot at every state change (after `subscribe()`). It is a listener: it keeps the plugin running | S:src/plugin/types/playback.ts:939-957 |

`plugin.player.on(event, fn)` listens to one event by name; it is a listener too, unless the event is a wake event (S:src/plugin/types/playback.ts:958-1022).

**The rule "a listener keeps its plugin running", as code** (P:src/host/src/capabilities/events.rs:83-95):

```rust
fn record(&self, host: &Host, owner: u64, _: &Value) -> Recorded {
    // …takes the event names this plugin subscribes to now: listened…
    if listened.iter().any(|event| !WAKE_EVENTS.contains(event)) {
        return Recorded::Live("an event listener");
    }
    // …only wake events: the subscriptions go into the recording…
}
```

The host calls this when it wants to put a plugin in standby. If the plugin subscribes to any event outside the wake events, the answer is `Live` and the plugin is not parked. This is the problem the design answers: a plugin that wants to know "a file was opened" can today only listen to `source.state`, ten times a second, and therefore stays up for ever. The rule itself stays; the design adds events and conditions that are not streams, so a plugin no longer has to listen to one.

**Today's 14 stream listeners** (T:"Today's listeners"; a static reading of the sources, not a runtime measurement). 11 of the 16 built-ins listen to `source.state`, and 3 of the 8 store plugins. What they use the stream for:

| Plugin | The moments it really needs | What else it holds |
| --- | --- | --- |
| vestra.history | Source change, source ready, pause, source end (with the old source's last position); a position save every 5 s | nothing |
| vestra.subtitle-online | Source change (abort old requests), source ready (automatic search) | nothing |
| vestra.window | Source ready with the video size known; "playing" and "fullscreen" flipping | nothing (but see F27) |
| vestra.info | Source change, only while its properties window or overlay shows | the window or overlay while shown |
| vestra.marks | Source change, source ready | a timeline layer |
| vestra.audio | Source change, source ready, audio tracks changed, output device changed | an audio filter chain, the adjust panel |
| vestra.subtitles | Source change, source ready, subtitle tracks changed | a subtitle style |
| vestra.video | Source change, source ready, dropped-frame rate over a threshold | a filter chain, shaders, the view |
| vestra.playlist | Someone else opened a file; natural end | the sequence provider |
| vestra.playback | Open failure, source change, the continuous position (seek bar) | the bottom-bar panel |
| vestra.visualizer | Source change, source ready, source end | the backdrop panel |
| zysilm.silenceskip | Source ready (scan); the position entering a silence (skip); the position going back | a timeline layer |
| zysilm.autoframe | Source change, source ready | its own crop filter chain |
| zysilm.loudness | Source change; continuous loudness readings (only while its window is open) | the window, an mpv filter and an observer |

The taxonomy's conclusions: none of the 14 needs the 10 Hz position stream for its once-per-file work; only displays consume continuous data (the seek bar, the information overlay, the loudness meter, the visualizer); 9 of the 14 hold other resources and stay Resident under today's rules even without the listener.

The listening code of three store plugins:

| Plugin | How it listens today | Source |
| --- | --- | --- |
| Silence Skipper | `subscribe()`, then `player.onChange(onPlayer)`; every 100 ms `onPlayer` checks whether the position is inside a silence not yet skipped and, if so, seeks to its end | G:plugins/silenceskip/main.ts:104,254-295,315 |
| Auto Frame | `player.onChange(onPlayer)` for source change and ready; once a crop is on, a 1 s timer reads back the shown size and withdraws the crop when another plugin changed the geometry | G:plugins/autoframe/main.ts:216,465-503,567-585 |
| Loudness | While its window is open: adds the `ebur128` filter to mpv's audio chain, hears `af-metadata/loudness` through `mpv.observe` (at most 10 values a second), and waits for a source change with `player.onChange` | G:plugins/loudness/main.ts:10-20,48-51,69,105-110 |

### 1.4 What `when` conditions can express today

`when` is a very small declarative language: a plugin writes "when is this command enabled, when is it checked" as a condition and hands it to the host, which evaluates it itself, so the menu follows playback while the plugin is in standby (P:src/host/src/conditions.rs:1-5).

```ts
plugin.commands.add({
  id: "left",
  path: ["Tools", "Time Left"],
  enabled: plugin.when.playing,   // on the wire: { all: [{ phase: ["ready"] }, { paused: false }] }
  run: async () => { /* … */ },
});
```

| No. | Fact | Source |
| --- | --- | --- |
| F17 | There are 7 atoms (the smallest tests): `phase`, `paused`, `kind`, `can`, `fullscreen`, `theme`, `language` | P:src/host/src/conditions.rs:11-16,29-40 |
| F18 | Composition is `all`, `any`, `not` only; a condition has at most 32 nodes, 6 levels | P:src/host/src/conditions.rs:24-26,95-101 |
| F19 | Wire form: a condition is an object with exactly one key | P:src/host/src/conditions.rs:102-105; S:src/plugin/when.ts:16-27 |
| F20 | The only use is a command's `enabled` and `checked`; any other field is refused | P:src/host/src/conditions.rs:208-217 |
| F21 | The facts conditions read all come from the player's own state: the last `source.state`, the last `window.state`, the theme, the language | P:src/host/src/conditions.rs:42-86 |
| F22 | The facts are rebuilt at every broadcast of `source.state`, `window.state` and `appearance.changed`; equal facts evaluate nothing, different facts evaluate every condition again | P:src/host/src/host/lifecycle.rs:474-479; P:src/host/src/commands.rs:1158-1178 |
| F23 | The semantics are level: the only answer is "does it hold now". No event is produced and no process is woken | P:src/host/src/conditions.rs:228-244 |
| F24 | The SDK has the same evaluation function, for the test host | S:src/plugin/when.ts:135-147 |

What cannot be expressed today (T:"when and latency"): any numeric comparison (position, duration, rate, volume, time); any equality on tracks, audio device, maximized, picture-in-picture; any "it changed". Of the taxonomy's 49 condition candidates, 8 can be written with today's atoms, 26 lack only an atom, and 8 are not tests on the current state at all (those can only be events).

### 1.5 What the host keeps for a plugin in standby, and what it drops

| Kept | Source |
| --- | --- |
| The recording's entry points (F3) | P:doc/architecture/supervisor.md:61 |
| The commands' `when` conditions, still evaluated, so the menu follows | P:src/host/src/conditions.rs:1-5 |
| The package's translation catalog, image roots, data directory, name | P:src/host/src/host/supervisor/standby.rs:47-63 |
| The triggers in its backlog | P:src/host/src/host/supervisor/standby.rs:1-4 |
| Session storage (`plugin.storage.session`): 1 MiB per plugin; survives standby, crashes, reloads, upgrades; cleared on disable, uninstall and player exit | P:src/host/src/session_storage.rs:1-12,30 |
| The files in its data directory | P:src/host/src/host/supervisor/standby.rs:16 |
| mpv properties it set with `mpv.set`, until the player quits | S:src/plugin/types/playback.ts:1890-1893 |

| Dropped (when the plugin stops) | Source |
| --- | --- |
| Everything in the process: variables, timers, promises in flight | T:"Gap groups" G4 |
| Every event subscription outside the wake events | P:src/host/src/capabilities/events.rs:105-110 |
| Its timeline layer | P:src/host/src/timeline.rs:1-3; P:src/host/src/host/lifecycle.rs:356 |
| Its always-on-top, cursor-hiding and picture-in-picture settings | P:src/host/src/host/lifecycle.rs:358-360 |
| Its labelled mpv filters and `mpv.observe` observers | S:src/plugin/types/playback.ts:1885-1893 |
| Its panels, windows, OSD panels | P:src/host/src/host/lifecycle.rs:337-348 |

Most of the second table is never actually lost today, because a plugin that holds one of these things is not put in standby in the first place:

| No. | Fact | Source |
| --- | --- | --- |
| F25 | The host answers "what does this plugin hold" in three ways: `Nothing`, `Calls` (registrations it can replay), `Live` (only the process can serve it: it stays running) | P:src/host/src/capabilities/mod.rs:94-104 |
| F26 | Holding a timeline layer is `Live`. Layers are kept by the process generation; a new source clears every layer | P:src/host/src/capabilities/timeline_marks.rs:20-25; P:src/host/src/timeline.rs:42-46,237-242 |
| F27 | Having written always-on-top or the cursor setting is `Live`. The test is "this plugin wrote a value"; writing `false` counts too, until it stops | P:src/host/src/capabilities/window_control.rs:26-33; P:src/host/src/ui/window_control.rs:22-32 |
| F28 | Resources in the playback thread are of two kinds. Held: filter chains, shaders, render options, a subtitle style, the video view, upscaling, motion smoothing, a labelled mpv filter or an observer; holding one makes the run Resident from then on. Work in flight: a resolution, a snapshot or job, a stream; it keeps the plugin up only while it lasts | P:doc/architecture/supervisor.md:63; P:src/host/src/playback/backend.rs:520-575 |
| F29 | A panel, an open window and an OSD panel are `Live` | P:src/host/src/capabilities/ui_mount.rs:65; P:src/host/src/capabilities/ui_window.rs:30; P:src/host/src/capabilities/ui_osd.rs:36 |
| F30 | A timeline layer holds at most 1000 marks and 100 ranges | P:src/host/src/timeline.rs:12,15; S:docs/reference/limits.md:121-122 |

## 2. The model

A plugin has four ways to get the player's state. The first three are discrete and let the plugin go to standby; the fourth is a stream and keeps it running, by the author's explicit choice. Nothing that exists is taken away in the first release: the model is "only add".

```text
            HOST                                               PLUGIN PROCESS (may be in standby: no process)
  playback thread: mpv + position model
  condition evaluator (the when language)   <-- 1 read once: request / answer -------  await player.read(), player.refresh()
  event table                               --- 2 discrete event (wakes) ------------>  player.on("source.ready", fn)
  supervisor: standby after 30 s idle,      <-- 3 condition handed over -------------  plugin.trigger().firesOn(condition)
     wake, backlog of 64                    --- 3 became true / false (wakes) ------>      .enter(fn)  .leave(fn)
                                            --- 4 state stream (keeps it running) -->  player.stream(fn)   ... .stop()
```

| Way | The question it answers | Started by | A plugin in standby | Example |
| --- | --- | --- | --- | --- |
| 1. Read once | What is the state now | The plugin asks, the host answers | n/a (the plugin runs) | Read the position after a wake |
| 2. Discrete event | What just happened once, with the data of that change | The host | is woken | Source ready; source ended, with the reason and the last position |
| 3. Trigger | When does the state become like this, and when does it stop being so | The plugin hands over a condition, the host evaluates it | is woken | Playing in fullscreen; the position is inside a silence; it is 23:45 |
| 4. Stream | What is the state at every change | The plugin opens it and closes it | cannot be in standby while it is open | A window that shows the time left |

| | Today | With this design |
| --- | --- | --- |
| A plugin wants to know "a file was opened" | `subscribe()`, then compares the source id in `onChange`, 10 times a second; so it is always up | `player.on("source.ready", …)`; woken once even from standby |
| A plugin wants to act while "playing in fullscreen" | The same stream, plus `window.state` | One trigger with that condition; `enter` and `leave` are called at the two flips |
| A plugin wants to know "it is 23:45" | A `setTimeout` in its process; lost with standby | One trigger with a time condition; the host wakes it then |
| A window wants the continuous position | `subscribe()` and `onChange(fn)`, for the whole run | `player.stream(fn)` while the window is open, `.stop()` when it closes |
| A plugin's marks on the seek bar, its always-on-top setting | Holding them keeps the plugin up | The host keeps them; the plugin can go to standby |

Third-party plugins avoid the stream wherever events and triggers can do the job (ruling 6): each of them is a process of its own, so a third-party plugin that parks frees a process. Built-ins share one process (F12); for them the gain is less event traffic and less JavaScript work.

## 3. The API

### 3.1 The whole API on one page

| Call | What it does |
| --- | --- |
| `await plugin.player.read()` | Exists. Reads the state once at start-up; `plugin.player.state` holds it afterwards |
| `await plugin.player.refresh()` | Exists. Asks for the state now and returns the snapshot |
| `plugin.player.on("event.name", fn)` | Exists; new event names. `fn` runs once per discrete event with that event's data (4.6) |
| `plugin.trigger(name?)` | **New.** Makes a trigger. The name is optional (4.2). Called only in the synchronous part of `activate()` (before its first `await`) |
| `.firesOn(condition)` | Hands the host a condition, or replaces the one the trigger has. Without a condition the trigger does nothing. May be written in the chain at the declaration, or called later |
| `.enter(fn)` | `fn` runs each time the condition turns from false to true |
| `.leave(fn)` | Optional. `fn` runs each time the condition turns from true to false |
| `.off()` | Removes the condition. The trigger stays declared and can get a new condition later |
| `plugin.player.stream(fn)` | **New.** Starts the state stream: `fn` runs with the new snapshot at every state change. Returns a handle. While a stream is open the plugin stays running |
| `.stop()` | Closes that stream. With no stream open the plugin can go to standby |

Removed in the second release, with no compatibility and no second spelling: `plugin.player.subscribe()` and `plugin.player.onChange()`. `plugin.player.state`, `plugin.mpv.observe` and everything else stay.

A condition is a value of the `when` language (4.3), for example `when.playing.and(when.fullscreen)`. It is data, not a function: the plugin hands it to the host and the host evaluates it, with or without the plugin's process.

`enter` and `leave` get one object:

| Field | Meaning |
| --- | --- |
| `state` | The player-state snapshot at the moment the host saw the condition flip |
| `range` | Only for a condition with `when.position.in(…)`: the range the position is in, `{ start, end, index }` |

**Across standby.** The host keeps each trigger's condition. When the condition flips while the plugin has no process, the host starts the process, `activate()` runs again and declares the same triggers, and the host hands the waiting notification to the same trigger. This is why triggers are declared synchronously: declared a step later, the notification has nobody to take it.

### 3.2 Example 1: on top while playing in fullscreen

The condition is known when the code is written, so it is given at the declaration. This is the window plugin's preference "on top while playing in fullscreen" (P:src/plugins/window/on-top.ts:27-36).

```ts
export default function activate(plugin: Plugin) {
  const { when, window } = plugin;
  plugin.trigger()
    .firesOn(when.playing.and(when.fullscreen))
    .enter(() => window.setAlwaysOnTop(true))
    .leave(() => window.setAlwaysOnTop(false));
}
```

| Line | In words |
| --- | --- |
| 1 | `activate` is the function the SDK calls each time the plugin's process starts |
| 2 | Takes two objects out of `plugin`: `when` (the condition language) and `window` (the main window) |
| 3 | Makes the plugin's first, and only, trigger. It has no name: the condition is handed over again at every start anyway |
| 4 | Its condition: "playing, and the window is fullscreen". The host receives it as data |
| 5 | When the condition becomes true: set the window on top |
| 6 | When it becomes false again: clear on top |

| Moment | What happens | Plugin process |
| --- | --- | --- |
| The plugin's first run | `activate` runs. Line 4 hands the condition to the host, which keeps it and evaluates it: false. After its idle time the host writes the recording and stops the process | exists, then none |
| Any later start of the player | The host has the condition and evaluates it after the start-up mute: false. Nothing is started | none |
| The user plays a film in fullscreen | The condition turns true. The host starts the process; `activate` runs; line 4 hands over the same condition again, which changes nothing; the `enter` function runs and sets on top | starts (measured 34–167 ms in the shared runner), then exists |
| 30 s later | The host stops the process. The on-top value stays in the host (4.8) | none |
| The user pauses | The condition turns false. The host starts the process; the `leave` function runs and clears on top | starts, then exists |
| The user resumes within 30 s | The condition turns true; `enter` is called in the process that is still there | exists |
| 30 s without a flip | Standby again | none |

### 3.3 Example 2: a sleep timer, with and without a name

The condition is known only when the user clicks, so it is given at run time. This is a hypothetical third-party plugin: the user picks "sleep in 45 minutes" and playback pauses then. Today it would need a `setTimeout` in its process and would have to stay up.

```ts
export default function activate(plugin: Plugin) {
  const { when, player, commands } = plugin;
  const alarm = plugin.trigger("sleep").enter(async () => {
    alarm.off();
    await player.setPaused(true);
  });
  commands.add({
    id: "in45",
    path: ["Tools", "Sleep in 45 Minutes"],
    run: () => alarm.firesOn(when.time({ min: Date.now() + 45 * 60_000 })),
  });
}
```

| Lines | In words |
| --- | --- |
| 3 | Makes the trigger and says what to do when its condition becomes true. No condition yet: nothing happens until someone gives one. `"sleep"` is its name |
| 4 | The alarm has rung: remove the condition, so it cannot ring a second time |
| 5 | Pause playback |
| 7–9 | Adds the menu item Tools ▸ Sleep in 45 Minutes |
| 10 | When the user clicks it: hand the host the condition "the clock is at or past now plus 45 minutes" |

| Moment | What happens | Plugin process |
| --- | --- | --- |
| 23:00, the user clicks the menu item | The click starts the plugin (F5). `activate` declares the trigger, then `run` hands the condition "clock ≥ 23:45" to the host. The host keeps it and compares the clock with 23:45 at each host tick (4.4) | starts, then exists |
| 23:00:30 | 30 s without a request: standby. The condition is in the host | none |
| 23:00–23:45 | Nothing. No timer runs in any plugin | none |
| 23:45 | The host tick sees the clock at 23:45 and the condition is true. The host starts the process; `activate` declares the trigger again, without a condition, so the kept one is left as it is; the `enter` function runs: `off()`, then pause | starts (estimated 0.15–0.3 s for a third-party plugin), then exists |
| 23:45:30 | Standby. The host holds no condition for this plugin any more | none |

**The same plugin without the name.** Line 3 would read `plugin.trigger().enter(…)`. Everything in the table above is the same. The difference shows in one case only:

| The user updates the plugin in the store at 23:10 | With `plugin.trigger("sleep")` | With `plugin.trigger()` |
| --- | --- | --- |
| What the host does with "clock ≥ 23:45" | Keeps it under the name `sleep` | Clears it: an unnamed trigger is known only by its place in the old version's code |
| 23:45 | The new version's `enter` runs; the film pauses | Nothing happens; the film plays on |

So: give a name when losing the condition at an update would be noticed by the user (4.2).

**The player is closed before 23:45.** The host evaluates conditions only while the player runs. Closed at 23:20 and opened again at 23:30: the condition is still there and rings at 23:45. Closed at 23:20 and opened the next morning: the condition "clock ≥ 23:45" is simply true at that start, so `enter` runs once and the morning's playback is paused. The host has no rule for this; the plugin judges it. Two ways to write it: an interval, `when.time({ min: due, max: due + 60_000 })`, which is false the next morning; or the plugin stores `due` in its own settings and its `enter` returns early when `due` is long past.

### 3.4 Example 3: Silence Skipper

The condition comes from a scan of the file. Today the plugin compares the position with its list of silences ten times a second (1.3) and so is always up.

The handler, declared in `activate`:

```ts
const silence = plugin.trigger().enter(async ({ range }) => {
  const now = await player.refresh();
  await sleep(((range.start - now.position) / now.rate) * 1000);
  await player.seek(range.end, { ifCurrent: now }).catch(ignoreStale);
});
```

| Line | In words |
| --- | --- |
| 1 | Makes the trigger. `enter` will be called with the silence that playback is about to reach, as `range` |
| 2 | Asks the host for the state now. The process may just have been started, which took time, so the snapshot that came with the notification is not used |
| 3 | One timer: waits until the silence starts. The distance is in media seconds, so it is divided by the playback rate. `sleep` is the plugin's own three-line helper around `setTimeout` |
| 4 | Jumps to the end of the silence. `ifCurrent: now` tells the host: do it only if nothing changed since `now`. If the user paused, dragged, changed the speed or opened another file meanwhile, the host refuses with `stale` (S:docs/guides/control-playback.md:50) and nothing happens; `ignoreStale` swallows that one error |

The rest of `activate`:

```ts
const layer = plugin.timeline.layer();
player.on("source.ready", async ({ state }) => {
  const spans = await scan(state);
  if (!player.isCurrent(state)) return;
  layer.setRanges(spans, { ifCurrent: state });
  silence.firesOn(when.playing.and(when.position.in(spans, { lead: 2 })));
});
```

| Line | In words |
| --- | --- |
| 1 | The plugin's layer on the seek bar |
| 2 | Runs once per file, when it is loaded. The event's data carries the state at that moment |
| 3 | The plugin's own ffmpeg scan; it takes seconds. `spans` is a list of `{ start, end, style }` |
| 4 | The user may have opened another file during the scan: then these silences belong to a file that is gone, and the handler stops |
| 5 | **Drawing.** Shows the silences on the seek bar. The host keeps and draws them from now on (4.8) |
| 6 | **Triggering.** The condition: "playing, and the position is inside one of these ranges or within 2 media seconds before one". Lines 5 and 6 are separate calls that happen to use the same list |

The author does not say which file the list belongs to. The host binds a range list to the source that is current when it receives the list, and the list stops matching when the source changes (4.3). `when.playing` is the author's choice: with it, a pause inside the 2 s makes the condition false and the resume makes it true again, so `enter` is called again; without it a pause would make the `seek` stale and that silence would not be skipped after the resume.

| Moment | What happens | Plugin process |
| --- | --- | --- |
| The user opens a lecture | The host sends `source.ready`; the plugin is in standby, so the host starts it. `activate` declares the trigger and the event handler; the handler runs | starts (estimated 0.15–0.3 s), then exists |
| The next seconds | The scan runs | exists |
| The scan is done | `setRanges`: the host draws the silences. `firesOn`: the host keeps the condition, bound to this file, finds the position outside every range, and computes when playback will reach the next range's start minus 2 s | exists |
| 30 s later | Standby. The silences stay on the seek bar; the condition stays in the host | none |
| Playback is 2 s before a silence | The host's deadline is due; it reads the real position once (4.4). The condition is true. The host starts the process; `activate` declares the trigger again (without a condition: the kept one stays); `enter` is called with that silence as `range` | starts, then exists |
| About 0.2 s later | Line 2: read the position. Line 3: a timer for the remaining 1.8 s or so | exists |
| The silence starts | Line 4: jump to its end. The position is now outside the range; the condition is false; there is no `leave` function, so nothing is called | exists |
| The next silence is less than 30 s away | `enter` is called in the process that is still there | exists |
| The next silence is further away | Standby after 30 s, then as from row 5 | none |
| The user drags back to before a silence | The position left the range and enters it again: `enter` is called again. No "seek" event is needed | as above |
| The user opens another file | The list stops matching. The new file's `source.ready` starts over from row 1, and its `firesOn` replaces the old condition | as row 1 |

What the sketch leaves out, and the remaining cost:

- The published plugin keeps a set of silences already skipped, so that a seek which lands on a keyframe inside the silence is not repeated (G:plugins/silenceskip/main.ts:53-54,254-294). The sketch does not need it: staying inside the same range is not a new `enter`. One case remains: if a seek can land *before* a short silence, `enter` would be called again and the plugin would loop; whether `player.seek` can do that was not checked, and if it can the handler needs the same guard as today.
- A speed change during the 2 s makes the `seek` stale and that one silence is not skipped. The result is a missed skip, never a wrong one.
- Where silences are less than 30 s apart the plugin is not parked during playback (standby needs 30 s without a request, F1, F2). It parks while paused, with no file, with a file without silences, with sparse silences. Gaps of 30–60 s are the worst case: every silence costs a process start. The host does not interfere (ruling 5).
- The lead of 2 s is "the worst wake latency plus a margin, times the rate". The wake latency of an own process is only estimated; a wake later than the lead makes the skip late or missed, not wrong. The reference migration measures it (plan A6, A7).
- Between line 4 and the host's receipt of line 6 the user can still open another file; the old list is then bound to the new file until the new scan replaces it. The window is one pipe write; it is the known cost of the implicit binding (§5, R13).

### 3.5 Example 4: a display window with the stream

A plugin with something on screen that follows playback continuously uses the stream, for as long as the thing is on screen. This hypothetical plugin shows the time left in a small window.

```ts
export default function activate(plugin: Plugin) {
  const { player, commands } = plugin;
  const view = plugin.windows.create("ui/left.slint");
  let feed: { stop(): void } | undefined;
  view.onClose(() => feed?.stop());
  commands.add({
    id: "left",
    path: ["Tools", "Time Left"],
    run: async () => {
      await view.show();
      feed?.stop();
      feed = player.stream((s) => view.set({ text: format(s.duration - s.position) }));
    },
  });
}
```

| Lines | In words |
| --- | --- |
| 3 | Declares a tool window from a layout file. It is not shown yet |
| 4 | A place to keep the stream's handle while one is open |
| 5 | When the user closes the window: stop the stream |
| 6–9 | The menu item Tools ▸ Time Left |
| 10 | Show the window |
| 11 | If the item is clicked while the window is already open, close the earlier stream first |
| 12 | Start the stream. The function gets the state now, and again at every change, and writes the remaining time into the window. `format` is the plugin's own helper |

| Moment | What happens | Plugin process |
| --- | --- | --- |
| The user clicks Tools ▸ Time Left | The click starts the plugin; the window opens; the stream starts and the function runs once with the current state | starts, then exists |
| While playing | The function runs about 10 times a second, as `onChange` does today | exists; the host does not park a plugin with an open stream |
| The user pauses | The function runs once for the pause, then not again until something changes | exists |
| The user closes the window | `stop()`: the stream ends | exists |
| 30 s later | Standby | none |

The same shape serves the built-in displays (the seek bar, the information overlay, the visualizer): they open the stream while their panel shows. They are unchanged apart from the new spelling.

## 4. Exact semantics

### 4.1 Triggers

| No. | Rule |
| --- | --- |
| T1 | **The host remembers one thing per trigger:** "not entered", or "entered" (for a range list: in which range). At each evaluation: true while not entered → `enter` once; false while entered → `leave` once. Nothing repeats while the condition stays as it is |
| T2 | **When it is evaluated.** When any fact the condition reads changes; for position and time, at the predicted moment (4.4). A pulse between two evaluations is not seen. One exception: a range that continuous playback passed through counts as entered and left once, even when it is shorter than the check step (as the HTML specification rules for `TextTrackCue`) |
| T3 | **Already true when given.** `enter` is called once, at once. So "read the state, then hand over the condition" has no gap in which a change is lost (ruling R9) |
| T4 | **The condition stays** until `.off()` or a new `.firesOn()`. There is no one-shot option; a handler that wants one shot calls `.off()` |
| T5 | **The same condition again changes nothing.** The host keeps "entered" as it is. This is what makes running `activate()` again harmless |
| T6 | **A different condition replaces the old one** in one step and is evaluated at once by T3; `leave` is not called for the old one. One exception keeps T1 (an `enter` is a real false-to-true edge): a range list replaced for the same source while the position is still in the range it was entered with (the same start, end and lead; its place in the list may move, as when a list grows with a scan) stays entered, with no second `enter` (found in A7: Silence Skipper re-gives its list as its scan goes on), and keeps its sequence number, so an `enter` of that range still on its way (on the pipe, or in the backlog of a plugin woken for it) is not dropped. That `enter` carries the range's place (`range.index`) in the old list, a later `leave` its place in the new one: `index` is the place in the list the host evaluated; `start` and `end` name the range. Every other `firesOn`, and every `off`, gets a new sequence number, and the SDK drops a notification that carries an older one |
| T7 | **`.off()`** removes the condition; `leave` is not called |
| T8 | **Declaration.** `plugin.trigger()` is called only in the synchronous part of `activate()`, and only on the plugin itself: the SDK throws when it is called later, or on a scope made with `plugin.scope()` (a trigger's condition is the plugin's and outlives any run, while a sub-scope is disposed within one; its kept condition would go on waking the plugin for functions that are gone). Declaring a trigger without `.firesOn()` leaves a kept condition untouched |
| T9 | **One trigger's handlers run one after another.** `leave` does not start before the promise of the preceding `enter` has settled |
| T10 | **Range lists.** Moving from one range straight into another is `leave` for the first and `enter` for the second. When several ranges match, the one with the earliest start is the matched one |
| T11 | **Seek, pause, rate change, buffering.** The position model is anchored again and the condition evaluated at once (4.4). A jump into a range is an `enter`, a jump out a `leave` |
| T12 | **A range list belongs to one source.** The host binds the list to the source that is current when it receives the condition. For every other source the atom is false. Such a condition is not kept across a player restart: the source it describes is gone |
| T13 | **A new process generation.** After the player starts, and after a plugin's process ended by a fault, the host forgets "entered" for that plugin, evaluates, and calls `enter` for what is true. After an ordinary standby it does not: "entered" is kept. There is no rule for what "was missed" while the player was closed: a condition that is true at the next start fires, and the plugin judges from its own stored data whether that is still wanted (ruling R11) |
| T14 | **Nobody to handle it.** A notification for a trigger that has neither an `enter` nor a `leave` function is dropped, its condition removed, and one diagnostic line written. When a run declares its triggers (`triggers.declare`, at the end of the synchronous part of `activate()`; sent only by a run that declares at least one trigger), a kept name that it did not declare is dropped; a different number of unnamed triggers than the host has kept for this version removes all the unnamed ones. Each writes a diagnostic line. A named condition kept from an older version whose new version declares no trigger at all stays until it first flips: that wake finds no trigger, the SDK says so, and the first rule removes it (one wake per such condition, once). It is not cleared at the ready instead: the SDK sends `triggers.declare` as a request, which may wait in its request queue behind the ready event under load, so "no declaration before the ready" does not prove "declares nothing", and a wrongly cleared condition (a user's alarm) is worse than one extra wake. A version built on an SDK before 1.10.0 never answers a notification, so such a condition stays with it until the plugin is disabled or uninstalled |
| T15 | **What does not exist** (ruling R6): no minimum interval, no cap on wakes, no refusal of an always-true condition, no loop detection |

### 4.2 A trigger's identity: declaration order, or a name

A plugin in standby has no process. What it left with the host is a condition, which is data. The functions (`enter`, `leave`) cannot be left with the host: they are code and exist only inside a running process. When the condition flips, the host starts a fresh process, which runs `activate()` and makes the plugin's triggers again. Something must say which kept condition belongs to which of the new triggers. Only two things are the same in the old process and in the new one:

| Identity | Written as | The host finds the trigger by |
| --- | --- | --- |
| Declaration order | `plugin.trigger()` | Its position among the unnamed triggers `activate()` makes. The author never sees the number. It is the same at every start as long as the code is the same and the triggers are made unconditionally |
| A name | `plugin.trigger("sleep")` | The name. It is the same also when the code changes, as long as the author keeps it |

A new version of a plugin is other code: a trigger put in front, or two swapped, and "the first trigger" is something else. The host cannot notice; it only ever knew a number. A condition that silently reaches the wrong function is worse than one that is lost. So:

| | Without a name | With a name |
| --- | --- | --- |
| Standby and wake; a plugin crash; a restart of the player; an upgrade of the player (same plugin version) | Kept | Kept |
| **The plugin is upgraded** (also: rolled back; reloaded with changed code during development) | **Cleared** | **Kept** |
| The new version does not declare it any more | – | Dropped when the new version becomes ready, with one diagnostic line |
| The plugin is disabled or uninstalled | Cleared | Cleared |
| What the author must observe | Declare unconditionally, in a fixed order (not inside an `if`, not in a loop whose length depends on data) | The name is unique within the plugin (a second trigger with the same name throws). Its meaning stays the same across versions |

"Another version" is judged as the recording judges it: by the package's digest and version (F4). Named and unnamed triggers may be mixed; the order counts only among the unnamed ones. Only kept conditions of unnamed triggers are cleared at an upgrade: never the plugin's settings and files, session storage, commands, key bindings, or the user's bookmarks.

A rule of thumb for authors:

| The condition … | Name? |
| --- | --- |
| is written at the declaration, in the chain inside `activate()` | No. Every start of the process hands it over again, and the new version is started once right after an upgrade (P:doc/architecture/supervisor.md:61,88) |
| describes the file that is open (a range list) | No. It is rebuilt for each file |
| is given later, because the user did something or because the plugin computed it, and should still hold after the plugin was updated (a timer, a reminder) | **Yes** |

A published name is a promise: the next version's function must be able to act on a condition an older version stored. If the meaning changes, use a new name.

**Precedent.** Chrome's extension alarms are timers kept by the browser that wake an extension's service worker. Its documentation (https://developer.chrome.com/docs/extensions/reference/api/alarms, section "Persistence", fetched 2026-11-14) says an alarm "persists until the extension updates" and advises: "it is best to make sure important alarms exists each time your service worker starts up". That is the behaviour of unnamed triggers. Chrome's alarms have names and are cleared at an update all the same, so a named trigger that survives an upgrade goes further than this precedent; its price is the promise above.

**How a plugin gets upgraded here.** Store plugins: the store checks by itself (30 s after start, then every 24 hours) but never installs by itself; an update happens when the user clicks "Update" (P:doc/architecture/store-design.md §3.6; P:src/plugins/store/store.ts:75-76). Built-ins change with a player release. So an unnamed trigger loses a pending condition only when the user updates that very plugin while the condition is pending.

### 4.3 The condition language

A condition is built from atoms and three ways to combine them: `a.and(b, …)`, `a.or(b, …)`, `a.not()` in the SDK (S:src/plugin/when.ts:65-77); `all`, `any`, `not` on the wire (F18). The wire form keeps today's rule (F19): an object with exactly one key. Numeric atoms are one half-open interval `{ min?, max? }`, meaning `min ≤ value < max`, with at least one bound.

| Class | In the SDK | Wire form | Meaning |
| --- | --- | --- | --- |
| The 7 existing | `when.ready`, `when.playing`, `when.fullscreen`, `when.can("seek")` … | `phase` `paused` `kind` `can` `fullscreen` `theme` `language` | Unchanged |
| Numeric | `when.duration({ min, max })`, `when.volume(…)`, `when.rate(…)` | `{ duration: { min, max } }` … | Duration in seconds (false when unknown), volume, playback rate |
| Identity | `when.audioTrack(id)`, `when.subtitleTrack(id)`, `when.audioDevice(id)`, `when.muted`, `when.maximized`, `when.pip`, `when.looping` | `{ audioTrack: 2 }`, `{ muted: true }` … | The selected audio track, the primary subtitle track, the active output device; muted, maximized (never while fullscreen, as `WindowState.maximized`), picture-in-picture, an A-B loop is set |
| Time | `when.time({ min, max })` | `{ time: { min, max } }` (Unix ms) | The wall clock is within the interval |
| Position list | `when.position.in(ranges, { lead })` | `{ position: { ranges: [[start, end], …], lead } }` | The position is inside one of the ranges, or less than `lead` media seconds before its start. False without a source and for any source but the one it was bound to (T12). A single point `t` is the list of one range `[{ start: t }]`, open at the end |

There is no `source` atom: an author never writes which file a condition is about (ruling R13).

**Two-valued logic.** A fact that cannot be read counts as false. So `when.position.in(…).not()` is true without a source.

**One language, two uses.**

| Use | Semantics | Result |
| --- | --- | --- |
| A command's `enabled`, `checked` (exists today) | Level: does it hold now | Menu state. The new atoms work here too, `subtitleTrack` included (the command registry gets the same facts as the triggers and follows the subtitle track's changes); `time` and `position` are refused with `invalid`: they need a clock and a source binding the command registry does not have |
| A trigger's `.firesOn()` (new) | Edge: the two moments at which it starts and stops holding | `enter` and `leave`; a wake when the plugin is in standby |

Level and edge are borrowed from epoll ([epoll(7)](https://man7.org/linux/man-pages/man7/epoll.7.html)).

**The range list.**

| Item | Rule |
| --- | --- |
| A range | Any object with `start` and, optionally, `end`, in media seconds; other fields are ignored. So the list given to `layer.setRanges` can be given to `when.position.in` as it is |
| `lead` | Optional, media seconds, the same for every range of the list. The range counts from `start − lead`. It makes up for the wake latency (4.9) |
| Overlap | Allowed. The matched range is the one with the earliest start |
| What the handler gets | `range: { start, end, index }`: the values as given, and the position in the list. It comes from the host, so it is there after a wake although the process is new |
| Per condition | At most one `when.position.in` atom, so that `range` has one meaning |

A range list is one node of the condition tree, however long it is; today's limit of 32 nodes and 6 levels (F18) neither counts nor limits the ranges. The host sorts the list once when it receives it, finds "which range is the position in" by binary search, and between two changes holds one deadline for the whole list.

**Limits.** 64 triggers per plugin; 1000 ranges per list, the same number as a timeline layer after the plan's step A5 (today a layer holds 100 ranges, F30). They are implementation details, listed in the limits reference like every other limit; a request beyond one is refused with `limit` (ruling R10).

### 4.4 How the host evaluates position and time

The host does not compare the position every 100 ms. Between two discrete events the position equals "the last anchor's position plus rate × elapsed time", and everything that breaks this (a seek, a pause, a rate change, a source change) is a discrete event the playback thread sees (T:"Continuous quantities"). So the host can compute when a boundary will be crossed. Only predictable state and the clock are evaluated this way; unpredictable signals (audio level, frame rate, cache, pointer) are not offered as conditions (ruling R7).

```text
position
   ^                                              o  deadline: read the real position once;
 X |- - - - - - - - - - - - - - - - - - - - - - -/-  at or past X -> report the crossing
   |                 pause: deadline       ___/
   |                 cancelled       _____/   resume: anchored again,
   |            ________________..../         t = t0 + (X - p0) / rate
   |      ___../
   |  o../   small dots: the playback thread's existing 100 ms reads, used only to refresh the anchor
   +--anchor (p0, t0, rate)-----------------------------------------------> wall clock
```

- **The model** lives in the playback thread: an anchor `(p0, t0, rate, running)`, where `running` means "ready, not paused, not seeking, not buffering". Predicted position `p(t) = p0 + rate × (t − t0)`.
- **Division of work.** The condition evaluator is on the core's thread (it also reads window and appearance facts, from the core's copy of the interface's facts). For each range list it gives the playback thread one number: the next boundary after the current position. The playback thread reports "crossed X, real position p, time t"; the core evaluates the whole condition, calls `enter` or `leave`, and hands over the next boundary. The playback thread does not know the condition language.
- **Deadline.** The playback thread's wait timeout changes from "the next 100 ms read" to "the earlier of the next read and the nearest boundary's deadline". At the deadline it reads mpv's `time-pos` once: at or past X, it reports; slightly short (clock drift, decoder lag), it waits for the remainder; not moving (a stall), it waits for the next anchor change.
- **Keeping the anchor fresh.** The playback thread reads the state every 100 ms anyway (P:src/host/src/playback/backend.rs:2144-2155); each read refreshes the anchor.
- **When the model is anchored again.** Seek finished, pause and resume, rate change, source change, A-B wrap, frame step, buffering start and end, end of playback. A buffering stall has no event today; mpv's `paused-for-cache` must be taken up. How far the playback thread handles each of the others today is checked item by item in step A2.

Where the error comes from (all estimates, §8): quantisation of `time-pos` (0 to one frame, 16.7–41.7 ms); timer granularity of the playback thread (about 15.6 ms at worst with Windows' default clock resolution); playback thread → UI thread → pipe (up to one frame); clock drift (at most 0.1 ms). In total the handler of a running plugin is called about 5–60 ms after the crossing; today's tick delay is 0–100 ms.

**Time.** The `time` atom compares the wall clock. While a time condition is kept, the host compares the clock with the nearest bound at each of the core's periodic steps (a 16 ms deadline on the core's own thread since player 0.10.2; before, the UI thread's 16 ms tick); the step is the timer, so a changed system time or a resume from sleep is seen at the next step, and a clock that went back evaluates every time condition again. Guarantee: never before the bound; normally within one step after it, later only while the core is busy (a stalled interface no longer delays it: K4 of [core-thread.md](core-thread.md), measured by `tests/ui_stall.rs`); due during sleep is seen at resume. There is no periodic atom: a handler that wants the next round calls `firesOn` again.

### 4.5 Reading once

`await plugin.player.read()` and `plugin.player.refresh()` exist and do not change (1.3). `read()` is the start-up read: after it `plugin.player.state` is current. `refresh()` is the read at any later moment and returns the snapshot. A plugin woken by an event or a trigger usually needs neither at first: the notification carries the state.

`plugin.player.state` stays and means "the newest snapshot this plugin has": from `read()`, `refresh()`, an event's data, a trigger's notification, the plugin's own calls, and, while a stream is open, every state change. Without an open stream it does not follow changes made elsewhere. Calls that act take `ifCurrent` as today, so a stale snapshot does nothing wrong: the host refuses with `stale`.

### 4.6 Discrete events

Events keep today's form, `player.on("event.name", handler)`. The name is a typed string: the editor completes it, a misspelt name is a compile error, and the handler's data type follows from the name. Every new event wakes a plugin in standby, so a listener of one does not keep its plugin running; they reach the plugin from the first listener.

A condition is a test on the current state. An event is one change, and its data carries what the current state no longer has, such as the old file's last position. That is why both exist.

**First batch (release A).** Each event's data carries `state`, the player-state snapshot at the moment of the event, in the SDK's own shape (`PlayerState`), so it can be passed as `ifCurrent`.

| Event | When it is sent | Data | What it replaces |
| --- | --- | --- | --- |
| `source.opening` | A new source starts to open (mpv's `start-file`) | `{ state, previous: { sourceId } \| null }` | Comparing the source id in the stream |
| `source.ready` | The source is loaded: duration, tracks and seekability known (mpv's `file-loaded`) | `{ state }` | Comparing `phase` and keeping "once per source" |
| `source.ended` | A source leaves playback: finished, stopped, replaced, failed, player quit (mpv's `end-file`) | `{ state, reason, error? }`; `reason` is one of `natural`, `stopped`, `replaced`, `error`, `quit`; `state` is **that source's** last state | `phase` and `endReason` in the stream |

For one source the order is `opening`, `ready`, `ended`; when a source is replaced, the old one's `ended` comes before the new one's `opening`.

**Second batch (release B).** A fixed next step once the first batch has proven itself in release A, not "when a plugin asks".

| Event | When it is sent | Data |
| --- | --- | --- |
| `video.reconfigured` | The decoded video's parameters changed, once to a few times per file (mpv's `video-reconfig`) | `{ sourceId, width, height, fps, rotation, hdr }` |
| `audio.reconfigured` | The audio parameters changed | `{ sourceId, sampleRate, channels, format }` |
| `audio.tracks.changed` | The audio track list or the selected track changed | `{ sourceId, audioTracks }` |
| `subtitles.changed` | The subtitle track list or the primary / secondary selection changed | The subtitle state |
| `window.changed` | Fullscreen, maximized or picture-in-picture flips; once when a resize gesture **ends**, not at every step | The window state |
| `layout.changed` | A panel slot's visible, active or detached state changed | `{ slots }` |
| `audio.devices.changed` | The output device list or the active device changed | `{ devices, active }` |
| `app.started` | Once per run: after start-up settled and the mute ended, at background priority | `{ version, previous }`; `previous` is the player version of the last run |

The exact data shapes of the second batch are fixed in step B1 against the built-ins that consume them; the implementation note below is that fixing.

#### Implementation note (step B1, SDK 1.11.0)

Written by the API maintainer with the implementation (2026-11-22). Host: `src/host/src/fact_events.rs` (names, the core's decisions), `src/host/src/playback/media_facts.rs` (the playback thread's), `SourceBackend::follow_device` (devices). SDK: `FactEvent`, `FactEventPayloads`, the third overload of `Player.on`.

**Data shapes**, as the listener of `plugin.player.on(name, fn)` gets them (`FactEventPayloads`). The wire shape is the same unless the last column says otherwise.

| Event | Data | Fixed against | SDK conversion |
| --- | --- | --- | --- |
| `video.reconfigured` | `{ sourceId, width, height, displayWidth, displayHeight, fps, rotation, hdr }`: `width`/`height` the decoded size (mpv `video-params/w,h`); `displayWidth`/`displayHeight` the size shown after aspect, rotation and the video filters (`video-out-params/dw,dh`, else `video-params/dw,dh`; 0 while unknown); `fps` the rate the file declares (`container-fps`), `null` when none; `rotation` degrees (`video-params/rotate`); `hdr` PQ or HLG transfer | vestra.window (auto-size reads the display size today through `player.stats()`, `video-size.ts`), vestra.video (coded size for the geometry editor, the source rate for 倍帧, the transfer for the HDR note, `main.ts` `readTransfer`, `measure`) | none |
| `audio.reconfigured` | `{ sourceId, sampleRate, channels, format }` (`channels` and `format` `null` while unknown) | vestra.audio (the channel count it probes twice after each track change, `readChannels`) | none |
| `audio.tracks.changed` | `{ sourceId, audioTracks }` | vestra.audio (the track menu, `rebuildTracks`; external track auto-load) | `audioTracks` as `AudioTrack[]`, the shape of `PlayerState.audioTracks` |
| `subtitles.changed` | `{ state }`, `state` the `subtitles.state` reply | vestra.subtitles (`subtitles.onChange` and the scan at open) | `state` as `SubtitlesState`, taken into `plugin.subtitles.state` first (usable as `ifCurrent`) |
| `window.changed` | `WindowState`: `{ fullscreen, maximized, videoArea: { width, height }, pictureInPicture: { supported, active } }` | vestra.window (on-top follows fullscreen, the picture-in-picture command follows `supported`, auto-size gives up when the user resized) | taken into `plugin.window.state` first |
| `layout.changed` | `{ slots: { bottom, sidebar, adjust, backdrop } }`; each position `SlotState` plus `panels` (`SlotPanel[]` in tab order, `active` marking the shown one), `null` without a panel; `backdrop` `{ effective }` or `null`. Wire: the `layout.state` reply | vestra.visualizer (`panels.backdrop.onChange`), vestra.video (the adjust panel's state) | wire → `{ slots }`, taken into the layout copy when it is in use |
| `audio.devices.changed` | `{ devices, requested, active }`: `devices` as `player.audioDevices()` lists them; `requested` the device `setAudioDevice` asked for this run, `active` the one in use (`null`: the system default), as `PlayerState.audioOutput` | vestra.audio (polls the list every 5 s and detects an unplug from `audioOutput`, `followOutput`) | none |
| `app.started` | `{ version, previous }`: this run's player version and the last run's (`null` on a profile's first run, and headless, which keeps no state file) | the store's update check | none |

**When each is sent, and where it is decided** (the core thread's rules, core-thread.md: no I/O on the core, interface facts only when settled, each event after what it derives from):

| Event | Decided on | Rule |
| --- | --- | --- |
| `video.reconfigured`, `audio.reconfigured` | The playback thread | mpv's reconfiguration marks them due; they are read after the iteration's `source.state` push of the same source went out, so a source's facts never precede its `source.opening`, and only once that source is loaded (`ready` or `ended`): while it loads mpv's parameters may still be the last file's chain or none, so a reconfiguration seen then (a late one of the last file included) waits and is read from the new file's parameters. Sent when the facts differ from the last ones sent for that source, and always for a new source's first; not without video (no decoded size) or without audio (no sample rate) |
| `subtitles.changed` | The playback thread | At each `source.state` push a cheap key (source, phase, selection stamp and targets, resources, mpv's track count) is compared; only when it moved is the whole state read. Sent for a loaded source (`ready`, `ended`) when its tracks or its primary or secondary selection differ from the last sent, including a new source's first |
| `audio.devices.changed` | The playback thread | The existing once-a-second device check reads the list every time (it read it only while a device was requested); the first check of a run is the start, not a change |
| `audio.tracks.changed` | The core | From two consecutive `source.state` broadcasts, after the source events they derive: the list when it is known and differs from the previous snapshot's (another source, a track added, the selection moved); not when it goes away |
| `window.changed` | The core | At `UiReport::FactsSettled` only. A flip (fullscreen, maximized, picture-in-picture `active` or `supported`) at once, with the size it has then; a size change alone after 300 ms without a further size change (the core's periodic step looks while one waits). The run's first facts are the start, not a change |
| `layout.changed` | The core | At `FactsSettled`: `layout.state` compared with the last one sent; the first of the run is the start. An overlay's pop-out is left out of the comparison: a position hidden only as an overlay (`reason` `fullscreen`, a column collapsed at the edge in fullscreen; `idle`, the fullscreen bottom bar away from the pointer) compares as shown, so the pointer revealing and hiding it sends nothing (a transient state, like a resize's steps for `window.changed`; as a wake event it would start the parked listeners at every hover). The data sent is the layout as it is |
| `app.started` | The core | Once, at the end of the start-up mute, after what the mute held was delivered (`wake_missed`) |

All eight are wake events (`FACT_EVENTS`). For the backlog a fact of one source merges by event name and source id (as the source events, §4.9), the others by name. For the start-up mute they are states, compared as the other wake events are (`OnDemand::announced`). A notification of one to a running listener counts as use, as a source event's does (§4.9).

**Decisions** (the implementing agent's, with reasons):

1. *The end of a resize gesture is 300 ms without a size change.* No portable signal exists: only Windows has `WM_EXITSIZEMOVE` (the host sees it, `platform/move_size.rs`), and a keyboard snap, a maximize animation or a layout change (a panel shown) sends none. A quiet period covers all of them on every platform; it is how browsers' resize handlers are debounced. A flip is not delayed.
2. *`video.reconfigured` carries both sizes.* The window plugin sizes the window to the shown size (after its own filters), the video plugin's geometry editor needs the decoded one; each read `player.stats()` for it. `fps` is the declared rate, which the video plugin's 倍帧 measures before its own frame blending.
3. *`audio.devices.changed` carries `requested` too.* The audio plugin tells an unplug from a choice by comparing `requested` with `active`; with `active` alone it would still need the stream.
4. *`subtitles.changed` and `window.changed` update the SDK's copy before the listener*, as the source events do for `player.state`: the data is then usable as `ifCurrent`, and a bound value that reads the copy is current after a wake.
5. *`app.started`'s "background priority" is its timing, not a new lane.* It is sent after start-up settled and its mute ended, when no start competes; a lane of its own would change the supervisor's trigger kinds for no measurable gain.
6. *`app.started`'s `previous` lives in the state file* (`ui-state.json`, `playerVersion`): written by the core's existing debounced write, once per run whose version differs. No new file, no I/O on the core.

**Removal of `subscribe()` and `onChange()` (step B2), as built.** The gate was the SDK's, not the host's: the host sends an event to every plugin subscribed to it, and the SDK subscribed the six events only after `subscribe()`. What replaces it (`Connection`, `Needs`): an event a plugin listens to is subscribed at its first listener. The six (`source.state`, `playback.backend`, `window.state`, `timeline.changed`, `appearance.changed`, `resolvers.fileTypes`) are subscribed only while something *holds* them: a plugin listener (`player.on`, `wire.on`), a watcher (`MainWindow.onChange`, a panel slot's `onChange`, `Subtitles.onChange`, a media handle's `ready` and `onEnd`, a scrub) or a stream. The SDK's own copy and a binding's read never subscribe them on their own, so a bound value or a command state that reads `player.state` shows the newest snapshot the plugin has (M3 (c)); while the group is held, a binding's read counts as a listener as before. When the last holder goes, the subscription ends and the copy stays as it was. A plugin that never called `subscribe()` sees no difference except that its listeners and watchers now hear their events (M3 (b)). `Media.ready` no longer polls every 100 ms: it holds `source.state` and reads the state once when the subscription starts.

**Late writes after `runtime.stop`.** From 1.11.0 the SDK refuses every request made after the plugin received `runtime.stop` (`RequestQueue.close`) with `disabled` ("… not sent: the plugin is stopping"), and sends nothing: the runner refused them already (`runtime/boot.ts`), and the player undoes what the plugin held itself. In the test host the plugin's own cleanup requests at its stop are therefore no longer sent either (the golden traces' `dispose` lists changed; the runner never sent them). What this does not change: a request the plugin wrote *before* the runner read `runtime.stop` reaches a host that removed the plugin already (`host/handle.rs`, the request is dropped unanswered) and is rejected with `disabled` when `runtime.stop` arrives. The store's lost `settings.set` of 11-21 was of that kind (S3b notes, `RESUME-S3b.md`): see the first item for the owner below.

**At quit.** A running plugin gets `source.ended` with `reason: "quit"` inside the existing second of quit time (F14). A plugin in standby gets nothing, as today: the host neither wakes it at quit nor stores the event for the next start (§6, M5).

**Left as they are.** The 11 existing events, including `window.state`, `playback.backend` and `timeline.changed`, and every `onChange` member other than `Player.onChange`.

**Not built** (ruling R12): system-environment events (display, power, session lock, network, resume from sleep), `vestra://` links, and the atoms that would go with them.

**Written as a trigger, not an event:**

| Need | Written as |
| --- | --- |
| A point t reached | `when.position.in([{ start: t }])` |
| Entering and leaving [a, b) | `when.position.in([{ start: a, end: b }])` with `enter` and `leave` |
| A played ratio; N seconds before the end | The plugin gets the duration at `source.ready`, computes the point, and uses the first row |
| The current chapter | The chapter list as a range list; `enter` gets the chapter as `range` |
| A clock time | `when.time({ min: T })` |

### 4.7 The stream

| Item | Rule |
| --- | --- |
| Start | `const feed = plugin.player.stream(fn)`. Synchronous; may be called at any time, not only in `activate()` |
| What `fn` gets | The new `PlayerState`. Once with the current state as soon as the SDK has it, then at every state change the player sends: every 100 ms while playing, at once when the phase, pause, rate or volume changes (today's `source.state`, 1.3) |
| Around each call | The SDK updates `plugin.player.state` first, then calls `fn`, then runs an update pass, so `bind()` functions and command-state functions that read `player.state` follow the stream, as with `onChange` today |
| Staying up | While at least one stream is open the host counts a listener and does not put the plugin in standby (the rule of 1.3, unchanged) |
| Stop | `feed.stop()`. A second call does nothing. A stream also ends when its scope is disposed or the plugin stops |
| After the last stop | The SDK ends its subscription. `player.state` stays at the last snapshot. Nothing of the stream keeps the plugin up: it goes to standby after its idle time if it holds nothing else (§6, M2) |
| Several streams | Allowed; each `fn` is called; the subscription ends with the last `stop()` |
| Across standby | Not kept. A plugin that is woken opens a stream again if it wants one |

**Release A** adds `stream(fn)` beside `subscribe()` and `onChange()`, which keep working unchanged. **Release B** removes those two; what else that removal touches is in §6, M3.

`plugin.mpv.observe` is unchanged: it stays the way to follow one mpv property, and holding an observer keeps the plugin up as today (F28).

### 4.8 Static state the host keeps through standby

Some things a plugin writes are plain data the host can keep and show by itself. Holding them no longer keeps the plugin running.

| State | Today | With this design |
| --- | --- | --- |
| The plugin's timeline layer: marks and ranges | `Live` (F26): the plugin stays up | Kept by the host through standby. Not counted as held |
| The window's always-on-top and cursor settings a plugin wrote | `Live` (F27), even after writing `false` | Kept by the host through standby. Not counted as held |

| Event | What happens to the kept state |
| --- | --- |
| Standby | Kept |
| After a wake | Still there; the plugin's next write replaces it as before |
| A new source | The timeline layer is cleared (unchanged), so a layer never outlives the source it describes. Window settings stay |
| The plugin stops after a fault | Removed. A faulted run is not trusted; the restarted run sets it again |
| Disable, uninstall, reload, upgrade; the player quits | Removed |
| Two plugins write the same window setting | The last write wins, as today |

Ownership moves from "the process generation" to "the installed instance" in three registries (layers, on-top, cursor). `timeline.changed` and its 100 ms throttle are unchanged.

**Not included** (ruling R14): Auto Frame's crop, which is two nodes in the plugin's own filter chain (G:plugins/autoframe/main.ts:206-207,221,260), and every other effect (filter chains, shaders, upscaling, motion smoothing, mpv filters). Holding one keeps the run Resident as today (F28). Effects kept by the host are a separate later topic.

### 4.9 Delivery and latency

| | The plugin runs | The plugin is in standby |
| --- | --- | --- |
| What it receives | One event line | The host starts the process; the notification waits in the backlog and is delivered at its ready, in the order the host saw |
| Priority | – | The "event" lane: below interactive triggers, above background starts (today's rule for wake events). It puts no other plugin into standby |
| Merging | None | Per trigger the backlog keeps the newest `enter` and the newest `leave`, in their order. A per-source event merges by "event name and source id", so two sources in a row keep one each; other events by name (today's rule, F8). The limit of 64 stays |
| Latency | One pipe write. Not measured | Shared runner (built-ins): **measured 34–167 ms**, median 82. Own process (third-party): **estimated 0.15–0.3 s**, not measured |
| Start-up | – | Muted during start-up and 1 s after (F9); evaluated and delivered when the mute ends |
| During a crash pause | – | Held for the start after the pause (F15) |
| While its idle standby is on its way (the host asks whether it holds anything, before the stop) | The notification counts as use, as a wake trigger to a running plugin does: the standby is cancelled and the process is not stopped under the handler. The three source events to a running listener do the same | Once the process is stopping, the notification is held for the next start, which it starts |
| It cannot be delivered (no process to take it, the plugin disabled or failed, its backlog or the start lane full) | One diagnostic line. "Entered" goes back to what the plugin last heard (a lost `enter` un-latches the trigger, a lost `leave` latches it again), so the next evaluation fires it again instead of the trigger staying quiet until its condition turns the other way and back; the plugin's next `triggers.declare` evaluates it too | The same |

**A notification that cannot be delivered** (no process to take it, the plugin disabled or failed, its backlog full) puts "entered" back to what the plugin last heard, so the next evaluation sends what it missed. Notifications of one evaluation may fare differently; when a `leave` is lost but the `enter` after it is delivered, the plugin never gets that `leave` (accepted: it learns the new range from the `enter`).

**The lead.** A wake is an order of magnitude slower than a frame. A plugin that must act at a point in the media asks to be told early: `lead` moves each range's start forward by that many media seconds. Woken, the plugin reads the position once, waits the rest with one timer of its own, and acts with `ifCurrent` (example 3). 2 s is the starting recommendation for third-party plugins, to be confirmed by the measurement of step A7.

### 4.10 What the host keeps, and for how long

The host keeps a plugin's conditions as it keeps the plugin's commands: on its side, without the plugin's process, and it evaluates them only while the player runs (ruling R11).

| What happens | Conditions of triggers | Event subscriptions |
| --- | --- | --- |
| Standby (idle, budget, memory pressure) | Kept, still evaluated | Kept (in the recording; today's wake-event mechanism) |
| The plugin crashes | Kept; "entered" is forgotten (T13); a notification during the pause waits for the next start | As today (F15) |
| The player restarts, same plugin version | Kept, except conditions with a range list (T12). "Entered" is not kept: after the start-up mute everything is evaluated once | Replayed from the recording |
| The player is upgraded | Kept: the plugin's code did not change | Recorded again (F4) |
| The plugin is upgraded, reloaded with other code, or rolled back | Unnamed: cleared. Named: kept; a name the new version does not declare is dropped at its ready (4.2) | With the new recording |
| The user disables or uninstalls the plugin | Cleared | Cleared |

On the pipe four messages carry the triggers (fixed in step A2): `triggers.set { trigger, when }` answered `{ seq }`, `triggers.off { trigger, unhandled? }` answered `{ seq }` (`unhandled`: the SDK had no `enter` or `leave` for a notification, T14), `triggers.declare { names, unnamed }` (sent only by a plugin that declared a trigger), and the event `triggers.fired { trigger, seq, edge: "enter" | "leave", state, range? }` (`state` in the wire shape with the position the flip was seen at; the SDK hands the handler a `PlayerState`). `trigger` is the name, or for an unnamed trigger its declaration order (a number), which the SDK fills in. The same condition given again answers the kept `seq` (T5), so a notification waiting in the backlog of a woken plugin is not dropped as older.

**Where the conditions are kept (M4, decided in step A2).** One small file per installed instance, `<package store>/triggers/<instance GUID>.json`: the conditions without a range list, each with its trigger and `seq`, the package digest and version that gave them, and how many unnamed triggers that version declares. It is written (a temporary file renamed over it) in the request that changed it, so a condition set 10 s before a player crash is there at the next start; it is never written when nothing it holds changed, and a range list is never written at all (T12), so the common case (a list per file, a chain condition given again at every start) writes nothing. It is not the standby recording and does not follow its lifetime: the recording is written only at standby and discarded at any package or player upgrade (F4). A process spawned from another package (digest or version) clears the unnamed conditions before it sends anything; named ones stay until its declaration drops a name it no longer declares. An installed run whose version cannot be told (it has no recording key: a built-in listed without a digest, a run not from the record's current package) is not judged another version: its conditions stay, and the version they were given by is not overwritten. "Entered" is never written (T13). Disable and uninstall delete the file. A development load keeps its conditions in memory only; its reload counts as another version. Code: `src/host/src/triggers.rs` (registry, evaluator, file), `src/host/src/host/triggers.rs` (facts, delivery), `src/host/src/playback/position_model.rs` (the anchor and deadline).

**Evaluation cost.** Conditions are indexed by the group of facts they read (the source's discrete facts, position, window, appearance, clock); a change in one group re-evaluates only the conditions that read it. Each range list and each time atom holds one deadline. With no position condition, the playback thread waits exactly as it does today. Worst case: 32 plugins × 64 triggers × 32 nodes, about 66 000 comparisons; estimated far below 1 ms, to be confirmed by a benchmark in step A2.

### 4.11 The test host and the docs

| Change | Content |
| --- | --- |
| Triggers | Conditions evaluated by the host's rules (`evaluate` in `when.ts` extended to the new atoms, F24); `host.triggers` shows what is kept; the SDK–host agreement test gains a shared table "condition × facts → truth" |
| A virtual clock and the position model | `host.advance(ms)` moves the clock and the playback position. A range's `enter` is called at its predicted time and tests do not wait |
| Standby and wake | `host.park()`: stops the plugin and keeps its subscriptions, conditions, layer and window settings; a later event or flip activates it again and delivers the backlog |
| Upgrade | `host.upgrade()`: applies the rule of 4.2 (unnamed cleared, named kept) and activates again |
| Discrete events | Opening, ending and changing tracks send the new events with the host's data |
| The stream | `player.stream(fn)` and `.stop()`, with the same "keeps it running" answer as the host |

All of these are additions to the test host. In release A no existing test or golden trace changes (V1). In release B every changed test and golden trace is listed in its pull request with the reason.

Docs: the SDK's events reference, messages reference (`triggers.*`), limits reference, the command-state guide (grown into the full description of the `when` language), a new guide with the four examples of §3, how plugins work, testing, troubleshooting ("why does my plugin not go to standby"); the player's `supervisor.md` §2, §4, §6, the events sections of the architecture overview, and a short section on the position model. General rules only, no page for one plugin.

## 5. Decisions, reasons, and alternatives set aside

All rulings are the user's, made on 2026-11-14 in the conversation with the main thread and recorded in the blackboard (`doc/process/rounds/history.md`, entries of 11-14). Where the reason was stated by the user it is quoted; otherwise it is the analysis the ruling was made on (drafts v1–v3).

| No. | Ruling | Reason | Alternative set aside |
| --- | --- | --- | --- |
| R1 | **The state stream stays**, renamed and reshaped: `plugin.player.stream(fn)` with `.stop()`. While a stream is open the plugin stays running | Displays need continuous data. A stream is the author's explicit choice, with a visible start and end | Draft v3: remove every stream; displays "read once" on their own timer (one request more per beat, D9 A), or a host-driven binding of panel properties (D9 B: a new host duty and a vocabulary that grows); remove `mpv.observe` (D8); remove or redefine `player.state` (D7); remove the old streams in one step as SDK 2.0.0 or deprecate first (D11). None of these questions exists any more |
| R2 | **`subscribe()` and `onChange()` are removed**, with no compatibility and no second spelling. They stay through release A and go in release B | One style only (ruling of ~17:30) | Keeping both spellings; a deprecation period beyond one release |
| R3 | **The model is "only add"**: read once, discrete events, triggers, and the stream. `player.state`, `mpv.observe` and the display-class plugins are unchanged apart from the rename | Nothing a plugin can do today is lost; each plugin moves when a better means exists | Draft v3's removal of `source.state`, `window.state`, `playback.backend` and the mirror subscription |
| R4 | **Triggers:** `plugin.trigger(name?)`, `.firesOn(condition)`, `.enter(fn)`, `.leave(fn)`, `.off()`; declared synchronously in `activate()` | The user found draft v2's `plugin.watch(name, fn).arm(condition, { data, once })` too hard to follow: "the Silence Skipper example made the plugin its own scheduler" | v2's `watch`/`arm`/`disarm`; `set` and `fire` as method names; a `data` blob handed over with the condition; a `once` option; one condition per moment instead of a range list |
| R5 | **The name is optional.** Unnamed: identity is the declaration order, the kept condition is cleared on upgrade. Named: it survives an upgrade | 4.2. An unnamed condition handed to the wrong function is worse than a lost one; a name is the author's statement that two versions mean the same thing | A name required for every trigger (v2); no names at all (ruling of ~17:30, superseded at ~18:40); keeping unnamed conditions when the new version declares the same number of triggers (unsafe: the same number does not mean the same order) |
| R6 | **No measures against polling.** No wake-rate limit, no ban on always-true conditions or loops | Staying up is the author's choice; the process budget (F13), the request cap (F16) and the memory-pressure rules remain the system's protection | Wake-rate limits as in Chrome alarms or Android |
| R7 | **Unpredictable signals are not for plugins** (audio level, frame rate, cache, pointer): no events, no conditions; read once stays | The host can evaluate only what it can predict from state and the clock without sampling | Derived aggregates from the host (mean level, dropped-frame rate); mpv properties as condition atoms |
| R8 | **Host-executed skip ranges (R1 of the plugin requests) are deferred** until measurements exist | Example 3 shows that a lead time does without; whether that is accurate enough is measured in step A7 | The host jumps "at a, to b" without waking the plugin. It is the fallback if the wake latency or the skip accuracy fails |
| R9 | **A condition already true when given calls `enter` once** | "The SDK does not babysit authors": one rule, one code path. Precedents: a timerfd set to a past time expires at once; epoll in edge mode reports a descriptor that is already ready | Not calling `enter` and returning `{ holds: true }` from `firesOn()` (two code paths for one thing; no caller to take the value after a restart) |
| R10 | **The caps (64 triggers per plugin, 1000 ranges per list) are implementation details** | They bound the work one message can put on the core and keep notifications from being dropped silently; they are not about how often a plugin is woken | Treating them as a design question under R6; no caps |
| R11 | **The host keeps conditions like it keeps a plugin's commands and evaluates them only while the player runs.** No rule for "missed while the player was closed": the condition is simply true at the next start and fires; the plugin judges staleness from its own stored data | "The SDK does not babysit authors" | Conditions that live only for one run of the player (every plugin with a trigger would have to be started at each start); dropping time conditions that came due while the player was closed (a daily check due at night would never run) |
| R12 | **No "delivered only while running" option for events.** System-environment events are not built now | No special machinery. No existing plugin uses the system group; an API is not extended ahead of a use | `player.on(name, fn, { wake: false })` (v3 D1); building display, power, lock, network events for three platforms without a consumer (v3 D10) |
| R13 | **A range list is bound by the host to the source that was current when it was given**, and stops matching when the source changes. The author does not write `when.source` | The approved line stays as short as it is: `silence.firesOn(when.position.in(spans, { lead: 2 }))` | An explicit `when.source(state)` atom combined by the author (v3 D12 A). Its advantage, no race when a scan finishes after the file changed, is given up; the author guards with `player.isCurrent(state)` (example 3, line 4) |
| R14 | **Static state kept by the host after standby:** timeline marks and ranges, the window's always-on-top and cursor settings. Auto Frame's crop is not included; effects are a separate later topic | Without the window settings the window plugin would stop parking at its first on-top write (F27). The crop is a filter chain, the class the host cannot restore by itself after a restart | Layers only; one rule that also covers "a geometry-only chain bound to the current source" |
| R15 | **A timeline layer only draws.** Drawing and triggering are separate calls | Two styles for one thing otherwise | A `notify` option on timeline layers |
| R16 | **Discrete events keep `player.on("event.name", fn)`**; events and triggers both exist | An event carries the data of one change, which the current state no longer has | Conditions only, plus a "value changed" atom |
| R17 | **Third-party plugins avoid the stream** wherever events and triggers can do the job | Each is a process of its own | – |
| R18 | **Versions:** first release SDK 1.10.0 / player 0.10.0 | – | SDK 2.0.0 for a one-step removal (v3) |

Further alternatives recorded in the drafts and set aside:

| No. | Alternative | Why it was set aside |
| --- | --- | --- |
| X1 | Let the existing high-frequency events wake | `source.state` every 100 ms means never parked |
| X2 | Plugins submit code to the host (as with eBPF) | Conditions are declarative: no functions, no code sent to the host |
| X3 | Position conditions compared at the internal 100 ms read | 0–100 ms late, as today. Kept as the fallback if the position model proves inaccurate |
| X4 | Conditions that are used up after one notification | A handler that wants one shot calls `.off()` |
| X5 | Conditions kept inside the recording file itself | A recording is discarded at a player or package upgrade (F4) and written only at standby; see §6, M4 |
| X6 | Debounce ("true for N ms before firing"), a "value changed" atom, a periodic atom, arithmetic between facts | Each needs history in the host beyond one truth bit; no migrating plugin depends on them |
| X7 | Three-valued logic: an unreadable fact is "unknown" and the condition does not fire | A more complex language. The fallback if two-valued logic surprises in practice |
| X8 | A "seek finished" event | Range lists cover "jumped over" and "jumped back" |
| X9 | A blocking hook before playback starts (the plugin decides where to start before the first frame) | A request, not an event; another topic. It is the real fix for "resume starts at 0, then jumps" |
| X10 | Timeline layers kept across a source change or a player restart | A layer describes the current source; keeping it leaves stale marks |
| X11 | The playback thread knows the condition language | Conditions also read window and appearance facts, which the interface reports to the core. The playback thread gets position boundaries only |
| X12 | Waking a plugin at quit to deliver `source.ended`, or storing the event for the next start (v3 D5) | §6, M5 |

**Later, decided from measurements:** host-executed skip ranges (R8); whether the 30 s idle threshold fits conditional wake (a plugin woken every 30–60 s costs a process start each time); effects kept by the host (R14).

## 6. Where the rulings meet today's code

Reading the code against the rulings showed five places where they do not fit as worded. Each has a decision by the maintainer, with its reason, so that the implementation has one answer. They are small enough not to stop the plan; the user may overrule any of them. M2 is the one that could grow into an architecture question and has a stop rule.

| No. | The ruling | What the code says | Decided here |
| --- | --- | --- | --- |
| M1 | "Read once (`await player.read()`)" | `read()` exists since SDK 1.9.0: it is the start-up read, returns nothing, and a second call reads nothing. The read that can be repeated and returns the snapshot is `refresh()` (1.3) | Both stay as they are. The examples use `read()` for the start-up state and `refresh()` for a later read. No rename: renaming either would be a second change of public API within two releases, for no gain |
| M2 | "After `.stop()` the plugin can go to standby" | A third-party plugin that holds a listener at its ready is classed Resident for that run (F11); only a listener added after the ready keeps the plugin up "only while it exists" (blackboard 11-14 ~13:00, finding D5e) | The design requires the ruling's behaviour in both cases: when the last stream stops, the host asks again what the run holds, and a run that holds nothing may park after its idle time. Step A4 determines how: if this is a change in the supervisor's adapter (asking again), it is done there; **if it needs a change to the supervisor core's state machine, the step stops and the question goes to the user.** Until then the documented pattern is example 4: open the stream when the thing that needs it appears |
| M3 | "`subscribe()` and `onChange()` are removed" | `subscribe()` does two more things besides feeding `Player.onChange`: it gates six events (`source.state`, `playback.backend`, `window.state`, `timeline.changed`, `appearance.changed`, `resolvers.fileTypes`), so that `MainWindow.onChange`, the panel slots' `onChange` and `Player.on` of those names hear nothing before it; and it keeps `player.state` current for `bind()` and command-state functions without any listener (the mirror, which keeps no plugin running) | In release B: (a) only `Player.subscribe()` and `Player.onChange()` go; every other `onChange` member stays. (b) The gate goes with `subscribe()`: each of the six events reaches a plugin from its first listener, as the ungated events do today; whether a listener keeps the plugin up is unchanged. (c) `player.stream(fn)` is what makes `player.state` follow outside changes; without a stream it is the newest snapshot the plugin has (4.5). A plugin that used `subscribe()` only so that a command's `enabled` function or a `bind()` follows the player writes the command state as a `when` condition, uses an event, or opens a stream. Each such place is found in step B3 and listed in its pull request |
| M4 | "The host keeps conditions like it keeps a plugin's commands" | Commands are kept in the recording: a file written when the plugin goes to standby, discarded at a player upgrade and at a plugin upgrade (F3, F4) | Read as the principle (kept on the host's side, replayed without the process, evaluated only while the player runs), not as "inside the recording file". Two things a recording cannot do are required: a named condition outlives a plugin upgrade, and a condition given at run time (the sleep timer) is written when it is given, not only at standby, so a player crash 10 s later does not lose it. The file layout is step A2's choice |
| M5 | The rulings do not say what happens to `source.ended` at quit for a plugin in standby (v3 D5) | At quit a plugin in standby receives nothing (F14) | No machinery, by the same reasoning as R11 and R12: neither a wake at quit nor a stored event. The consequence falls on vestra.history, which needs the last position: when it is migrated in release B it opens the stream while a file is open (from `source.ready` to `source.ended`) and keeps its 5 s save, so nothing is lost at quit or at a crash; it parks when no file is open. As a built-in it shares a process, so staying up during playback costs no process (F12) |

Two smaller points of the same kind:

- **Event data.** `Player.on` hands a listener the wire shape of an event today ("the player's wire shape, not the SDK's view of it", S:docs/reference/events.md). The new events carry `state` as the SDK's `PlayerState`, because the examples pass it as `ifCurrent` and to `isCurrent`. The SDK converts it; the typed payload table says so per event.
- **Loudness.** Draft v3 said the loudness meter parks after its window closes. By today's rule a plugin that held an mpv filter or an observer is Resident for the rest of that run (F28; blackboard 11-14 ~13:00, finding B3, the intended supervisor design). That is unchanged: effects kept by the host are the later topic.

## 7. What each plugin uses afterwards

The rows of the 11 built-in and 3 store listeners come from the taxonomy and were checked against the sources for draft v2; the other store plugins' rows come from a text search, not from a run. Step "baselines" of the plan checks all of them against the "keeps running" reasons in the host log before any migration.

| Plugin | Uses | Afterwards |
| --- | --- | --- |
| vestra.history | `source.ready`, `source.ended`; the stream while a file is open (M5) | Parks with no file open |
| vestra.subtitle-online | `source.opening`, `source.ready` | Parks |
| vestra.window | `source.ready`, `video.reconfigured`, `window.changed`; one trigger with `enter` and `leave` (example 1) | Parks (through 4.8) |
| vestra.info | While its window or overlay shows: `source.ready` and its existing 1 s timer, or the stream | Parks when nothing shows |
| vestra.marks | `source.ready`; the "current chapter" check as a range list | Parks (through 4.8); it has more listeners, to be checked |
| vestra.audio | `source.opening`, `source.ready`, `audio.reconfigured`, `audio.tracks.changed`, `audio.devices.changed` | Resident: audio filter chain, adjust panel |
| vestra.subtitles | `source.opening`, `source.ready`, `subtitles.changed` | Resident once a subtitle style is set; else parks |
| vestra.video | `source.opening`, `source.ready`, `video.reconfigured`; the dropped-frame rate keeps its own timer | Resident: filter chain, shaders, view |
| vestra.playlist | `source.opening`, `source.ended` with `natural` | Resident: sequence provider |
| vestra.playback | `source.ended` with `error`, `source.ready`; the stream for the seek bar | Resident: bottom-bar panel (display class) |
| vestra.visualizer | `source.ready`, `source.ended`, `layout.changed`; the stream while it shows | Resident: backdrop panel (display class) |
| capture, hotkeys, preferences, store, workshop | Unchanged (`app.started` for the store's update check) | Park today |
| zysilm.silenceskip | Example 3 (reference migration, release A) | Parks when silences are more than 30 s apart |
| zysilm.autoframe | `source.ready` | Parks on files without borders; Resident while a crop is on (R14) |
| zysilm.loudness | `source.opening`; `mpv.observe` unchanged | Parks until its window is first opened; Resident for the rest of that run (§6) |
| zysilm.clipexport | Read once; the selection layer through 4.8 | Up during an export; parks otherwise (to be checked) |
| zysilm.shaderupscale, zysilm.amdmotion | The needless `subscribe()` goes | Resident: shaders, motion mode |
| zysilm.vestratube, zysilm.vestranes | Read once | Park today; up while their stream plays |

**The bound on the benefit.** Built-ins share one process (F12); parking them saves event traffic and JavaScript work, not a process. Only third-party plugins save a process. 9 of the 14 listeners hold other resources and stay Resident.

## 8. Estimates not yet measured

No measurement was made for this document. Each of the following appears in the text as an estimate, as arithmetic, or as the result of a static reading. The plan's step that turns it into a measurement is named.

| No. | Statement | Nature | Measured in |
| --- | --- | --- | --- |
| E1 | A third-party plugin's own process wakes in 0.15–0.3 s | Estimate, a sum of parts (T:"Wake latency") | Preparation (P3); A7 |
| E2 | A range boundary is reported, and the handler called, 5–60 ms after the crossing | Estimate | A2, A7 |
| E3 | Clock drift is at most 0.1 ms | Arithmetic on the assumption "of the order of one in a thousand" | A2 |
| E4 | mpv's `time-pos` advances per frame when there is video | An assumption about mpv, not checked | A2 |
| E5 | The worst case is about 66 000 comparisons, far below 1 ms. **Measured in A2** (`triggers_sim.rs` `worst_case_evaluation_cost`, release build, this laptop): 2048 conditions of 32 nodes, one in four with a 1000-range list, all evaluated by one change: 1.1 ms when nothing flips, 2.0 ms when all 2048 flip (the notifications included). Not "far below 1 ms": about 1–2 ms on the UI thread for a case no real set of plugins approaches | Arithmetic and estimate | A2 (benchmark) |
| E6 | A 2 s lead is enough; in example 3 the timer is set for "about 1.8 s" after a wake of "about 0.2 s" | Depends on E1 | A7 |
| E7 | Silences in lectures are often less than 30 s apart | An assumption, not counted | A7 (does it go to standby) |
| E8 | Today's state broadcast to the built-ins is more than a hundred messages a second | Arithmetic (11 listeners × 10 Hz) | Preparation (P2), B5 |
| E9 | Every "afterwards" cell of the table in §7 | A static reading of the sources | Preparation (P2); A7; B5 |
| E10 | A resume that comes 0.1–0.2 s late shows as "from 0, then a jump" | The taxonomy's judgement | B5 (GUI check) |
| E11 | The host can obtain the last position when a source ends | **Settled in A1**: yes, without a change to the playback thread. The core derives the three events from consecutive `source.state` broadcasts (`src/host/src/source_events.rs`) and gives `source.ended` the last broadcast of that source: exact at a natural end and a failure (the broadcast that reports it) and while paused; when a playing source is replaced or closed, at most one broadcast interval (100 ms of playback) old | A1 |
| E12 | The playback thread sees every event that re-anchors the position model except a buffering stall. **Checked in A2**, against `backend.rs`: a seek (a plugin's, a step seek, a keyframe seek, an A-B wrap and a file loop, which mpv performs as seeks) is mpv's `seek` then `playback-restart`, both in its event loop; pause and resume are the existing `pause` observation; a rate change and buffering are the `speed` and `paused-for-cache` observations A2 adds (only while a position condition is watched); a source change is the generation of every read; a frame step is the `player.step` request or a core `frame-*` action; the end is the phase of every read. Any jump that comes without one of these is caught at the next 100 ms read (further than 0.3 s from the prediction); a stall that mpv does not report as buffering is a running read that does not move for 250 ms, after which the model waits for movement instead of a deadline | A reading of the taxonomy, not checked item by item | A2 |
| E13 | When the last stream stops, the host can re-classify the run without a change to the supervisor core (M2) | **Checked in A4: only for a stream opened after the ready.** Such a run was classified lazy; the stream is a listener that the idle check finds (`stays running`), and after the last `stop()` the SDK marks the subscription a mirror again (or ends it), so the next idle check parks the run: no change anywhere in the host. A stream open at the ready makes the run resident (`Supervisor::classified`), and the core has no transition from resident back to lazy within a run (`arm_idle` skips a resident unit; `classified` accepts one answer per run; `holds_live` goes only the other way). Parking it after `stop()` needs a new core transition: **stop point, brought to the user** | A4 |
| E14 | A plugin upgrade while a user-set condition is pending is rare | A judgement from how updates are installed (4.2) | – |
| E15 | Delivery to a running plugin is "one pipe write" | No figure | A7 |

The only measured values quoted are: shared-runner wakes of 34–167 ms (51 wakes); `node -e 0` starting in 78–103 ms. Both come from the taxonomy and were not repeated.

## 9. What could not be verified

- Every example is a sketch. None was compiled against the SDK or run. The existing names were read in the SDK's sources (`player.on`, `player.read`, `player.refresh`, `player.isCurrent`, `player.seek` and `player.setPaused` with `ifCurrent`, `plugin.window.setAlwaysOnTop`, `plugin.timeline.layer().setRanges`, `plugin.windows.create`, `view.show`, `view.set`, `view.onClose`, `when.playing`, `when.fullscreen`, `.and()`); the new ones do not exist. `PlayerState.position` and `duration` can be `null` with nothing open; the sketches leave that check out.
- Whether `player.seek` always lands at or after the requested position (the loop case in 3.4).
- That built-in plugins change version only with the player, and that the store has no "update all" action (4.2): a search of the docs, not a proof.
- The taxonomy's listener table: the rows of silenceskip, autoframe, loudness, history and window were compared with their sources; the other 9 plugins' rows were not re-read one by one.
- The user's reasons in §5 are as recorded in the blackboard and in the brief for this document; a reason given only in conversation and recorded in neither is not here.

## 10. Glossary

| Word | Meaning |
| --- | --- |
| `activate()` | The function of a plugin that the SDK calls each time the plugin's process starts. Its synchronous part is everything before its first `await` |
| Standby (the code also says park) | The process is stopped, the entry points stay |
| Wake | Starting a plugin in standby again |
| Backlog | Where things that woke a plugin wait until its process is ready. 64 per plugin |
| Recording | The file that lists the entry points a plugin registered, used to rebuild them without starting the process |
| Resident | A third-party plugin that is not put in standby because it holds something only its process can serve. This document says it of built-ins too |
| Live, held | The host's two words for what a plugin holds. Live: only the process can serve it (a panel, a window, a listener). Held: a resource in the playback thread that the plugin's next activation takes up again from its own settings (a filter chain, shaders) |
| Installed instance | The identity of one installation. It does not change when the process restarts or the plugin is upgraded |
| Source, source id | The file or stream that is open. Each open adds one to the id; the same file opened again gets a new id |
| Event | A message the host sends to a plugin on its own |
| Stream | The state sent at every change, about ten times a second while playing. `player.stream(fn)` |
| Discrete event | An event sent once per thing that happened |
| `when`, condition, atom | A declarative test handed to the host for evaluation; an atom is the smallest test |
| Level, edge | Level: does it hold now. Edge: the moment it starts or stops holding |
| Trigger | A place for one condition and the functions to call when it becomes true (`enter`) and false (`leave`) |
| Kept condition | The condition a trigger was last given through `.firesOn()`, kept by the host so that it can evaluate it without the plugin's process |
| Range list | The list given to `when.position.in`: ranges of media time, one atom |
| Anchor | In the position model, "at this time the position was this and the rate was this", from which later positions are computed |
| Lead | Media seconds by which each range of a list starts earlier, to make up for the wake latency |
| `ifCurrent`, `stale` | A state snapshot sent with a request; the host refuses the request with the error `stale` if the source changed or anyone paused, sought or changed the speed since |
| Display class | A plugin with a panel or window on screen that follows playback continuously |
| V1, V2, V3 | The project's checks: static gates, the runtime differential, the frame-rate measurement ([verify-additions.md](../process/verify-additions.md)) |
