# Events and triggers: execution plan

**Status: plan, nothing started.** Written 2026-11-14 from the user's rulings of that day. The design is [events-and-triggers.md](../architecture/events-and-triggers.md) (called "the design" below); it is the authority for what is built. This file says in what order, by whom, and how each step is known to be done. The running state is in the blackboard, [rounds/history.md](rounds/history.md).

## 1. How the work is run

| Item | Rule |
| --- | --- |
| Workflow | [agentic-workflow.md](../../agentic-workflow/agentic-workflow.md) §5 (top-down), run under [agentic-workflow-loop.md](../../agentic-workflow/agentic-workflow-loop.md) |
| Main thread | Architecture, with the user |
| Deputy coordinator | Runs the lanes: briefs, commits, reviews, merges, GUI checks, the blackboard |
| Agents | One role per agent. Sub-agents never commit and never start agents |
| Implementing agents | The API maintainer, plugin authors (migrators), and build/test verification: **Opus 5.5** |
| Static review | **Fable, medium effort.** Reads the design, this plan, the diff and the code. No builds, no logs. Judges one thing: did the implementer understand the documents and the intent of the architecture, and implement that correctly |
| Build/test verification | **Opus.** Re-runs the checks of the stage itself (V1–V3 and the stage's own tests) and reports the results |
| When reviews happen | **At stage boundaries only** (A1, A2, … each is a stage), not after each small change. After fixes, a brief re-read of the changed parts, not a second full review |
| Fixes | Go back to the agent that wrote the code |
| Open points | The implementing agent decides open design items and gives its reasons. Coupling, a new host duty or bloat goes to the user before the merge |
| Repositories | SDK work on a branch of the SDK repository; the player depends on a pre-release tag of it (`v1.10.0-rc.N`) until the release (workflow §5.1). No force pushes |

For this project the two-part review at stage boundaries replaces the loop's "independent review that re-runs every check" after every commit (loop §4). It is the user's ruling for this plan and changes no workflow file.

## 2. Verification

| Check | What | When |
| --- | --- | --- |
| V1 static gates | `src/scripts/verify-gates.mjs` in each repository touched ([verify-additions.md](verify-additions.md)) | Every merge. Release A: all gates pass, no `--allow`. Release B: each changed test and golden trace is named in the pull request with its reason |
| V2 runtime differential | `src/scripts/verify-runtime.mjs compare` against the staged 0.9.0, four scenarios, built-in and store plugins | Every merge that touches the host, the SDK runtime or a plugin. **Intended differences are only which plugins go to standby**; each stage states its expected table before the run (§6), and any difference outside it fails |
| V3 frame rate | The frame-rate measurement with `VESTRA_FRAME_LOG`, CPU rendering included: the interface reaches the display rate | Every merge that touches the playback thread, event delivery or a display-class plugin |
| GUI checks | By the coordinator, in an isolated profile, never the user's | Where a step names them |
| Full `ci:local` | `npm run ci:local -- --strict-sdk` | At the end of each release (A7, B5), before the release step |

A step is done when its done-check holds and the blackboard says so. A check that cannot be run is reported as not run, never as passed.

## 3. Preparation

| Step | Content | Owner | Done when |
| --- | --- | --- | --- |
| P1 Archive | The design and this plan are in the repository, linked from `doc/architecture/README.md`; the blackboard records the rulings of 2026-11-14 and points here | Maintainer writes, coordinator commits | Both files are on main; the blackboard entry exists |
| P2 Baselines | `ci:local` on main; the V2 run of the staged 0.9.0 with the store plugins (its `summary.txt` is also the inventory of why each of the 24 plugins keeps running); the V3 baseline | Build/test verification | `ci:local` green on main and its commit recorded; `normalised.json` and `summary.txt` of 0.9.0 saved and their folder named in the blackboard; V3 figures recorded |
| P3 Third-party wake latency | Measure, from host logs, spawn to ready of a store plugin in its own process (installed from a file, at least 30 wakes): minimum, median, p90, maximum | Build/test verification | The four figures are in the blackboard and replace estimate E1 of the design. **If p90 is over 0.5 s, the 2 s lead and example 3 are brought to the user before A2 starts** |

## 4. Release A: SDK 1.10.0 / player 0.10.0, additions only

Nothing that exists changes behaviour, except what the design names: which plugins go to standby (A3, A6), the range limit (A5) and the listed fixes (A5). Existing tests and golden traces pass unchanged.

| Step | Content | Owner | Done when |
| --- | --- | --- | --- |
| A1 First batch of events | `source.opening`, `source.ready`, `source.ended` (design 4.6): sent by the host, wake events, typed in the SDK with `state` as `PlayerState`; per-source merging in the backlog; `reason: "quit"` to running plugins only; test host sends them | Maintainer | Host and SDK tests cover the order `opening` → `ready` → `ended` for open, replace, failure, stop and natural end, and a wake from standby by each; `source.ended` carries the ended source's last position (estimate E11 settled); V1 passes; V2 shows no difference |
| A2 Triggers | The condition atoms (numeric, identity, time, position list); `plugin.trigger(name?)`, `.firesOn`, `.enter`, `.leave`, `.off`; the host evaluates from predictable state and the clock (the position model of design 4.4, the timers of the `time` atom); kept conditions, optional names, clearing on upgrade (design 4.1–4.3, 4.10); the caps; the test host's virtual clock, `park()` and `upgrade()` | Maintainer | Unit tests of the evaluator and a pure simulation of the position model with the time passed in: entering, jumping over, jumping back, pause, rate change, a very short range, adjacent ranges, source change, a stall; rules T1–T15 each have a test; named kept, unnamed cleared at an upgrade; the SDK–host agreement table "condition × facts → truth" passes on both sides; the benchmark of the worst case is recorded (E5); each re-anchoring event of design 4.4 is checked against the playback thread and listed (E12); V1 passes; V2 shows no difference; V3 shows no regression |
| A3 Static state kept | Timeline marks and ranges, the window's always-on-top and cursor settings are kept by the installed instance through standby and no longer count as held (design 4.8) | Maintainer | Tests: kept through standby, replaced after a wake, removed at a fault, disable, uninstall, reload and upgrade; layer cleared at a new source; last write wins between two plugins. V2: the only differences are plugins that held nothing but these now going to standby, as listed beforehand |
| A4 `player.stream(fn)` | Added beside `subscribe()` and `onChange()`, which keep working unchanged (design 4.7); the first call with the current state; `.stop()`; after the last stop the run may park (design §6, M2) | Maintainer | Tests: `fn` gets the current state, then every change; an update pass follows each call; an open stream keeps the plugin up; after `.stop()` a plugin that holds nothing else parks after its idle time, both when the stream was opened after the ready and when it was open at the ready. **If the second case needs a change to the supervisor core's state machine: stop, bring it to the user.** V1 passes; V2 shows no difference |
| A5 Pending SDK fixes | mpv observe listeners get an update pass; a second observer of the same property gets the current value; stale docs (the `font-feature` rule in the design rules and `doc/product/ui-spec.md`, the `mpv` capability text, the unrendered template in the mpv reference, internal plan paths in the public CHANGELOG, the residency of listeners and of mpv filters and observers); `i18n extract` fails loudly when the SDK's types do not resolve; `AudioTrack.streamIndex`; the timeline range limit unified with 1000 (host `MAX_RANGES`, SDK `TIMELINE_LIMITS.ranges`, the trigger's range list); `vestra-plugin check` compiles `.slint` | Maintainer | Each item has a test that fails before and passes after, or for a doc item the corrected text; the three range limits read 1000 from one constant per repository; V1 passes with additions only; the items are listed in the CHANGELOG |
| A6 Reference migration | Silence Skipper, by the maintainer personally (workflow §5.1 P1): design 3.4, no `subscribe()`, no `onChange()`, no stream | Maintainer | `check`, `build`, `test` pass; its tests run on the test host with `park()`; a grep for `subscribe(` and `onChange(` in its source returns nothing; behaviour as today in its own test list; line counts before and after reported |
| A7 Reviews and measurements | The static review and the build/test verification of the whole release; V1–V3; full `ci:local`; GUI measurements with Silence Skipper on a real lecture file: does it go to standby (when, and why not), skip accuracy (position at the seek against the silence's start, at 1× and 2×), wake latency (spawn to ready, and trigger to handler) | Reviewers; coordinator for the GUI | Both reviews say merge; `ci:local` green; the measurements are in the blackboard and the design's estimates E1, E2, E6, E7, E15 are replaced by them. **Stop point: if the measurements contradict the design (a skip that is late or missed at a 2 s lead, a wake over 0.5 s at p90, a plugin that never parks where the design says it does), stop and bring it to the user** before A8 |
| A8 Release | SDK 1.10.0 (tag, release workflow, npm); player 0.10.0 on the released SDK; Silence Skipper published; release notes; the blackboard | Coordinator | npm `latest` is 1.10.0; the player release has its assets; the store shows the new Silence Skipper; `target-wt-*` folders of merged worktrees removed |

Order: A1 before A2 (the evaluator's notifications use the same delivery); A3, A4 and A5 are independent of each other and of A2; A6 needs A1, A2, A3 and A5. Each stage is one branch and one merge.

## 5. Release B: the next version, unified spelling

The version numbers are set at B6. Removing `subscribe()` and `onChange()` is a breaking change of the SDK's public API; whether that makes it SDK 2.0.0 is the user's call then.

| Step | Content | Owner | Done when |
| --- | --- | --- | --- |
| B1 Second batch of events | `video.reconfigured`, `audio.reconfigured`, `audio.tracks.changed`, `subtitles.changed`, `window.changed`, `layout.changed`, `audio.devices.changed`, `app.started` (design 4.6). A fixed step once the first batch has proven itself in release A, not "when a plugin asks" | Maintainer | Each event has host and SDK tests and wakes from standby; the data shapes are fixed against the built-ins that will consume them and written into the design; `window.changed` is sent once at the end of a resize gesture; V1 passes; V2 shows no difference |
| B2 Remove `subscribe()` and `onChange()` | `Player.subscribe()` and `Player.onChange()` leave the SDK and the test host; the gate on the six events goes (design §6, M3); no compatibility, no second spelling | Maintainer | The API surface lists exactly these two removals; a grep for either in the SDK's source, docs and examples returns nothing; an old plugin that calls one fails at activation with a message that names `player.stream(fn)`; every changed test and trace is listed with its reason |
| B3 Migration | All 16 built-ins, the 8 store plugins and the SDK examples (design §7). Third-party plugins leave the stream wherever events and triggers can do the job; built-ins use the new events instead of diffing the stream; display-class plugins use `player.stream(fn)` while they show; history keeps the stream while a file is open (design §6, M5) | One migrator per plugin, at most 4 at a time; small plugins first (workflow §5.2, §5.3) | Per plugin: `check`, `build`, `test` pass; no `subscribe(` or `onChange(` of the player in its source; behaviour unchanged (same menus, windows, effects); every place that read `player.state` without a stream is listed with what it uses now; its V2 row matches the table stated beforehand; a migration blocker is reported, not worked around |
| B4 Docs | The SDK's references (events, messages, limits), the command-state guide grown into the `when` language, a new guide with the four examples, how plugins work, testing, troubleshooting; the tutorials; the Workshop prompt and scaffold; the player's `supervisor.md` and architecture overview | Maintainer; the Workshop's author for its prompt and scaffold | No page mentions `subscribe()` or `Player.onChange`; every example in the docs compiles in the docs build; a plugin made from the scaffold uses no stream unless it shows something; general rules only, no page for one plugin |
| B5 Reviews and checks | The static review and the build/test verification of the whole release; V1–V3; GUI checks: the seek bar is as smooth as today, on-top follows fullscreen playback, resume does not show "from 0, then a jump", the history position survives a quit and a crash; full `ci:local` | Reviewers; coordinator for the GUI | Both reviews say merge; `ci:local` green; V2 differences are exactly the stated standby table; V3 reaches the display rate under CPU rendering; the GUI list passes |
| B6 Release, cleanup, records | SDK and player released; store plugins published; worktrees and `target-wt-*` removed; the blackboard round folded into the history; `doc/status.md` | Coordinator | Released versions recorded; no leftover worktree or target folder; the history has the round |

## 6. The intended differences of V2

Stated before each run; anything outside the table fails.

| Release | Plugin | 0.9.0 | Expected |
| --- | --- | --- | --- |
| A (A3) | A plugin that holds only a timeline layer or a window setting and listens to no stream | keeps running | may be stopped while idle |
| A (A6) | zysilm.silenceskip | resident (an event listener, a timeline layer) | recorded; woken by `source.ready` in the `file` scenario; parked again when idle |
| B (B3) | vestra.history, vestra.subtitle-online, vestra.window, vestra.info (nothing shown), vestra.marks | keep running | may be stopped while idle (history: only with no file open) |
| B (B3) | zysilm.autoframe (no crop), zysilm.loudness (window never opened), zysilm.clipexport (no export) | resident or kept by a listener | may be stopped while idle |
| B (B3) | Display and processing class (playback, visualizer, audio, video, playlist, shaderupscale, amdmotion) | keep running | unchanged |

The rows are the design's static reading (its estimate E9). P2's inventory corrects them before A3 and B3 state their final tables.

## 7. Later, decided from measurements

| Topic | Decided from |
| --- | --- |
| Host-executed skip ranges (the host jumps "at a, to b" without waking the plugin) | A7's skip accuracy and wake latency |
| The 30 s idle threshold | How often a plugin is parked and woken in A7 and B5 (a plugin woken every 30–60 s costs a process start each time) |
| Effects kept by the host (filter chains, shaders, mpv filters, Auto Frame's crop) so that their plugin can park | A separate design topic after release B |

## 8. Stop points

| Where | Stop when | Then |
| --- | --- | --- |
| P3 | A third-party wake is over 0.5 s at p90 | The lead and example 3 go to the user before A2 |
| A4 | Parking after `.stop()` needs a change to the supervisor core's state machine | To the user (architecture rule) |
| A7 | The measurements contradict the design | To the user before A8 |
| Any step | A change couples components, adds a host duty the design does not name, or bloats | To the user before the merge |
| Any step | A migration blocker: behaviour that the new API cannot express | Reported to the maintainer; the migrator does not invent an alternative |
