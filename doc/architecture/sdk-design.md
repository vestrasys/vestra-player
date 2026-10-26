# SDK design notes

Why the plugin API (SDK v2) is designed the way it is: handles, lifecycle, update passes, how the SDK absorbs protocol details, conditional submission, and the error model. Written for API maintainers and plugin authors who want a deeper understanding; the exact interface definitions are in the [API reference](https://vestrasys.github.io/vestra-player-sdk/modules/_vestra-player_sdk.html), and how to write plugins is in the [plugin examples](https://vestrasys.github.io/vestra-player-sdk/documents/Examples.html).

Code: the npm package `@vestra-player/sdk`, in the separate repository [vestra-player-sdk](https://github.com/vestrasys/vestra-player-sdk). SDK paths below are relative to that repository: public types are in `src/plugin/types.ts`, exported from the package entry `src/index.ts`; the test host is `src/plugin/testing.ts` (`@vestra-player/sdk/testing`); `createPlugin` is in `@vestra-player/sdk/runtime` (for the player only); unit tests are in `test/`; the build tool is `bin/vestra-plugin.mjs`. In the player repository: the runner is `src/runtime/runner.ts`, and the shortest complete plugin is `src/plugins/video/`.

## 1. In one sentence

A plugin default-exports `activate(plugin)`. Factories on `plugin`, grouped by domain, create **handles** (commands, windows, panels, filters, media, snapshots, jobs…). A handle carries its own callbacks and its own last-write-wins state sync, and is released automatically with its scope. Plugin code only describes "what it wants"; all protocol fields stay inside the SDK.

```ts
import type { Plugin } from "@vestra-player/sdk";

export default async function activate(plugin: Plugin) {
  const filter = plugin.video.filter("picture");
  const window = plugin.windows.create("picture.slint");
  window.on("adjust", (field: string, value: number) => filter.set(nodesFor(field, value))); // nodesFor: the plugin's own mapping
  plugin.commands.add({ id: "video.picture", path: ["Video", "Picture Adjustments…"], run: () => window.show() });
}
```

## 2. Modules and handles at a glance

`Plugin` is the root scope; a `Scope` has the same modules (handles created by a child scope belong only to that child scope).

| Module | Factory / members | Handle | Operations on the handle | Plugins using it |
| --- | --- | --- | --- | --- |
| `commands` | `add({ id, path?, title?, order?, group?, radio?, bindings?, focus?, repeat?, enabled?, checked?, busy?, reason?, run })` | `Command` | `set(state)`, `ready` | all |
| `windows` | `create(file, { id? })` | `Window` | `on`, `set`, `bind`, `show`, `hide`, `close`, `opened` | video, audio, subtitles, capture, nes, bilibili, youtube, workshop, playlist |
| `panels` | `mount("bottom" \| "sidebar", file, { component?, global?, title? })`; `bottom`, `sidebar` (slot state) | `Panel`; `PanelSlot` / `SidebarSlot` | Panel: `on`, `set`, `bind`, `setInteraction`, `showMenu`, `ready`; Slot: `state`, `onChange`, `setVisible`, `toggle`, `setDetached` (sidebar) | playback, playlist |
| `dialogs` | `openFile`, `openFiles`, `openDirectory`, `saveFile` | — (returns a path or `null`) | | playback, playlist, subtitles, capture, nes, youtube, workshop |
| `player` | `state`, `onChange`, `refresh`, `isCurrent`, `open`, `setPaused`, `seek`, `setRate`, `restart`, `close`, `scrub`, `setOutput`, `openStream`, `provideSequence`, `next`, `previous`, `stats` | `Media`, `Scrub`, `Stream` | Media: `current`, `ready`, `onEnd`, `close`; Scrub: `seek`, `active`, `end`; Stream: `running`, `onStop`, `stop` | playback, playlist, audio, capture, nes, bilibili, youtube |
| `video` / `audio` | `filter(id)` | `VideoFilter` / `AudioFilter` | `set`, `status`, `onStatus`, `retry` | video, audio |
| `subtitles` | `state`, `onChange`, `refresh`, `select`, `load(bytes, { ifCurrent?, signal? })`, `remove`, `setStyle` | `SubtitleTrack` (value object: `id`, `kind`, `embeddedId?`, `title`, `language`, `styleSupported`, `styleReason`, `owned`) | | subtitles |
| `media` | `capture({ at?, processed?, subtitles?, audio?, ifCurrent? })` | `Snapshot` → `ExportJob` | Snapshot: `export`, read-only description; Job: `state`, `onChange`, `done`, `cancel`, `refresh` | capture |
| `resolvers` | `add({ id, schemes, hosts, resolve })` | `Resolver` | `ready` | bilibili, youtube |
| `packages` | `validate(dir)` (`rollback(id)` removed in 1.5.17) | `PackageCandidate` → `PackageChange` | Candidate: `install`; Change: `status`, `message`, `onChange`, `done`, `cancel` | workshop |
| `osd` | `show(text)`, `panel()` | `OsdPanel` | `set(text \| null)` | info |
| Scope | `scope()`, `add`, `onDispose`, `onError`, `update`, `dispose` | `Scope` | | all (`onError`, `update` as needed) |

The manifest capabilities each module needs (the host grants by capability; the SDK does not change that) are on the SDK documentation site, [Reference: Capabilities](https://vestrasys.github.io/vestra-player-sdk/documents/Capabilities.html) (generated from the SDK's `CAPABILITY_INFO`, and checked by `src/tests/sdk/host-agreement.test.ts` against the host's per-method checks in `dispatch`). Without the events capability, `media.ready()` polls the source state automatically.

Subtitle notes: compare subtitle tracks by `track.id` (an opaque string; do not parse its format); the track number of an embedded track is `track.embeddedId` (external tracks have no such field). The `signal` of `load` is checked between chunks and before the commit: if already aborted, the upload is discarded and the call rejects with `AbortError`; once the commit has been sent it is no longer aborted (a commit already handed to the host may have taken effect). Upload ids start at 1 for each plugin instance: the host assigns a new owner generation each time it loads a plugin, and removes the owner's upload high-water mark when the owner is released.

Naming choices: user-visible domain nouns (`player`, `filter`, `panel`, `stream`, `snapshot`, `job`, `resolver`); host-internal names (`processing`, `producer`, `source`, `mount`, `$mount`) do not appear in the plugin API. There is no `ctx`: the object a plugin receives is simply called `plugin`, and a feature module receives a `Scope` or a specific handle.

## 3. Lifecycle and release

1. **Activation**: the runner loads the entry module. It first creates the plugin object (with `playback.read` it reads the source state once; with `events` it subscribes to `source.state`), calls `activate(plugin)`, then waits for all registrations started during activation (commands, panel mounts, resolvers, event subscriptions) to finish, and only then reports `runtime.ready`. If any registration fails, activation fails (`runtime.failed`). Plugin code does not need to `await` these registrations.
2. **Declaring is registering**: `commands.add`, `panels.mount` and `resolvers.add` return a handle synchronously, and registration is queued in the background; `await handle.ready` when needed. For handles created after activation, registration failures go to the `onError` of their scope.
3. **Scopes**: every handle joins the current scope when created. `scope.dispose()` releases all its handles and child scopes in reverse creation order; calling `handle.dispose()` directly removes it from its scope. Releasing twice is harmless, and release never throws (failures are only logged).
4. **Plugin deactivation**: the host sends `runtime.stop` → the runner refuses new requests and releases the root scope. By then the host has already revoked all of the plugin's resources by owner (menus, windows, panels, processing resources, subtitle resources, snapshots, jobs, resolvers, producers), so release only does local cleanup (stopping timers, aborting signals, closing producer sockets), and requests to the host are rejected immediately and ignored. A plugin does **not** need to return a cleanup function.
5. **What each handle does on release** (applies when a child scope is released while running):

| Handle | Release |
| --- | --- |
| Command | No longer dispatched; `commands.unregister` removes it from the menu and binding table (a re-registration with the same id is ordered after the removal) |
| Window | `ui.close` |
| Panel | `ui.unmount` |
| Sequence provider (`provideSequence`) | `sequence.set(null)` |
| OsdPanel | `osd.panel(null)` if it has shown text |
| VideoFilter / AudioFilter | If it was ever applied, replaces this resource with an empty chain (only withdraws its own contribution) |
| Snapshot | `snapshot.release` |
| ExportJob | `jobs.cancel` if not finished |
| Stream | Stops rendering, closes the source if it is still current, releases the producer |
| Resolver | `sourceResolvers.unregister`, aborts resolutions in progress |
| Listeners (`onChange`, `onEnd`, `on`, `bind`…) | Unsubscribe |

`Media` does not close the source with its scope: when the plugin is deactivated, current playback continues (the playlist's existing behaviour). Call `media.close()` explicitly when it should close.

## 4. State sync: values, bindings and update passes

Continuously changing values only describe the desired state; the SDK merges, de-duplicates and sends them in order:

- `window.set(props)` / `panel.set(props)`: writes the desired value immediately.
- `window.bind(() => props)` / `panel.bind(...)`, and functions in command options (`enabled: () => ...`): **bindings**, re-evaluated in every update pass.
- **Update pass**: the SDK schedules a pass automatically at these points (several requests within one microtask run only one pass): after any plugin handler called by the SDK (command `run`, window/panel callbacks, `onChange`, `onStatus`, `onEnd`, `onStop`, etc.) returns synchronously and after its promise settles; after changes to the player, subtitles, panel visibility, filter status or export job status. When a plugin changes state in its own timer or in the middle of a long task, it calls `plugin.update()`.
- Each target has only one request in flight at a time; changes in the meantime are merged into the next send. A `bind` value is sent only when the **desired value changes**; a value written with `set()` is always sent once (even if identical to the previous one), so `set()` is also the explicit way to "pull a control the user changed back to the model value". Callbacks themselves do not trigger a resend: in-out properties the user changed in Slint (input text, tab, slider position) stay unchanged after other callbacks.
- Write failures: a binding value that failed is not resent automatically until it changes; a `set()` value is retried with backoff (starting at 0.25 s, doubling, up to 30 s); a run of consecutive failures reports only the first error. So writing state to a broken window inside `onError` does not create a retry loop.
- A window opens on its first `show()` with the current desired properties as its initial properties, so default values never flash first; `show()` after `close()` reopens it with the latest properties.
- Name conversion: TS uses camelCase (`effectEnabled`, `toggleEffect`), and the SDK converts to Slint's kebab-case; names already in kebab-case are used as is. Struct fields (such as `rowId` of a list row) are not converted.

This is a deliberately small "reactive" system: no dependency tracking, no virtual DOM, only "evaluate, compare, send changes". Plugin state is plain variables.

## 5. How protocol details are absorbed

| Host protocol concept | What the SDK does | What the plugin sees |
| --- | --- | --- |
| `revision` counter | Commands: 1 at registration, incremented on every state change; processing resources: `max(local, current host value)+1`; subtitle style: incremented locally | Nothing |
| `expectedGeneration` / `expected` | Processing resources: reads `processing.state` before each submission and uses the current source generation; source control: by default uses the source stamp of the `player.state` the plugin sees; open: unconditional by default; subtitles: by default uses the selection stamp read before submission; snapshots: `player.state` by default | Nothing by default; `{ ifCurrent: state }` when needed (section 6) |
| `operationId` and `*.operation(...)` | One increasing counter per domain (separate operation books for source, subtitles and media); the host's reply is the final result; only when "the reply is lost" does it query by the same id (see below) | A Promise: done / failed (`code` + displayable `message`) / pending (not finished yet) |
| `unknown` / `unknownExpired` | See the table below | Never appears |
| `eventSequence` / generation comparison | `player.state` only accepts states whose sequence number is not lower than the one already delivered; identical content is not delivered twice. Subtitle state drops older generations, or older revisions within the same generation | `onChange` only receives newer states |
| UI name conversion, per-property `ui.set`, callback allowlist | Section 4; the callback allowlist when a window opens = the handler names bound before opening (binding a new callback name after opening throws, telling you to bind before the first `show()`); for panels the host forwards all callbacks of the global, with no allowlist | Nothing |
| `backendUncertain` | Checked before a filter submission; when the stage is blocked by the host, the status is `failed`, `error.code === "backend-uncertain"`, and `message` is the host's reason; `retry()` checks again | A single `filter.status` |

### 5.1 Handling "result uncertain", and why it works

The grounds (all existing host behaviour):

1. **The pipe is ordered**: the host reads requests from the plugin's stdout on a single thread, in order; processing, source, subtitle and media requests then enter background serial worker threads in order. So a query sent after a request is always handled after that request has been accepted or rejected.
2. **The host replies to every request**, or stops the plugin process (in which case the runner rejects all pending requests and the plugin no longer needs a conclusion).
3. A host rejection (RPC error, such as `stale`, `busy`, `unsupported`) is a **definite** result: the request was not executed, or its failure has been recorded.

So the runner **sets no client-side timeout on requests**. Timeouts were the main source of "result unknown" before: the client gave up waiting at the deadline but could not know whether the host had executed the request. Now a request taking more than 10 seconds only writes one log line and keeps waiting for the real reply.

The SDK still handles "lost replies" (a rejection that is not a host error, or a client `timeout`), with these rules:

| Domain | Query | Conclusion |
| --- | --- | --- |
| Processing resources | `processing.operation(stage, resourceId, revision)` | `completed` → applied; `failed` → failed; `unknown` → by rule 1 it was never accepted and certainly not executed, so resend with a new revision (fail with `unknown` after 3 in a row); `unknownExpired` → read `processing.state`: if this resource's current revision equals that revision it was applied, otherwise it did not take effect and is resent; `pending` → query again after 100 ms |
| Source | `source.operation(id)` | `pending` means accepted and still running (e.g. URL resolution in progress); keep querying at intervals starting at 40 ms, up to 500 ms, until `completed`/`failed`; if the query itself is rejected by the host → fail with `unknown`. Each plugin can only query its own operations; the host accepts `playback.control` or `playback.read` (`required_capability` in `main.rs`) |
| Subtitles | `subtitles.operation(id)` | `completed` → adopt its state; `failed` → failed; `unknown` → fail with `rejected` (not executed); `unknownExpired` → refresh the state and fail with `expired` |
| Snapshots/jobs | `jobs.operation(id)` | `completed` → adopt the result (a snapshot then reads `snapshot.state` again); `failed`/`unknown`/`unknownExpired` → failed; `pending` → query again after 250 ms |
| Package install/uninstall | `packages.operation(requestId)` | The host guarantees `operationId === requestId`; keep querying until a terminal state, never resend |

Under v2 these branches almost never happen in practice (there are no client timeouts), but keeping them means the SDK still gives a definite conclusion when the transport misbehaves, and **never blindly resends a non-idempotent operation** (open, subtitle commit, starting a job, install). Only last-write-wins processing resources can be resent safely, and only when they were definitely not executed.

The root cause of the audio workbench getting stuck on "result of the previous operation awaiting confirmation" (the plugin treated `unknown` as a state needing user confirmation and blocked all updates meanwhile) does not exist in v2: the SDK decides by rule 1 that `unknown` was not executed and handles it, and the plugin only sees `applying` → `applied`/`failed`.

## 6. Explicit conditional submission

Default semantics: **an operation applies to the state the plugin sees**.

- Source control (`setPaused`, `seek`, `setRate`, `restart`, `close`) and `media.capture` are conditioned on `player.state` by default: if another plugin or the user has changed the source since then (open, pause and seek all advance the source stamp), the call fails with `code: "stale"` and does nothing. The SDK refreshes `player.state` after each of its own operations completes, so consecutive operations of your own do not make each other stale.
- `player.open` replaces the current source **unconditionally** by default (as a user-initiated open should).
- Subtitle selection, removal and style are conditioned by default on the current selection read before submission.
- Processing resources are always conditioned on the current source at submission time (the host requires it); this is transparent to the plugin.

To bind to an earlier snapshot, keep a `PlayerState` (or `SubtitlesState`) and pass `{ ifCurrent: state }`:

| Scenario | How to write it | Plugin |
| --- | --- | --- |
| Auto-advance after natural end: open the next item only if nobody changed the source after it ended | `media.onEnd((ended) => player.open(next, { ifCurrent: ended }))` | playlist |
| Snapshot: freeze the source of the frame the user saw before confirming | `const seen = player.state; …confirm…; media.capture({ ifCurrent: seen })` | capture |
| Dragging the seek bar: each seek in one gesture is conditioned on the result of the previous one, and any external change ends the gesture | `const scrub = player.scrub(); scrub.seek(t)` | playback |
| Subtitle style applies only to the track selected when the user started adjusting | `subtitles.setStyle(style, { ifCurrent: editing })` | subtitles |
| Check only, without submitting | `player.isCurrent(state)` | capture (warns during a form that the source has changed) |

For example code, see "Playback bar", "Playlist", "Snapshot export" and "Subtitles" in the usage documentation.

## 7. Error model

- Every failure rejects/throws a plain `Error` with a stable `code` (type `PluginError`): host error codes are kept as is (`stale`, `busy`, `unsupported`, `unavailable`, `conflict`, `invalid`…); a `{ backend }` source operation failure maps to `code: "backend"` with the backend's text as `message`; the SDK's own codes are `replaced`, `failed`, `ended` (`media.ready`), `aborted` (`AbortError`), `backend-uncertain`, and `unknown`/`rejected`/`expired` (see 5.1).
- Error codes of `player.open()` / `openStream()`: `stale` (with `ifCurrent`, the source had changed and nothing was done); `replaced` (the host accepted it, but before it finished it was replaced or withdrawn by another source, e.g. the user opened another file while the URL was still resolving; same meaning as `replaced` from `media.ready()`, not a failure of this item); `backend` (loading/resolution failed, `message` is the backend's text); `invalid`, `busy`, `conflict`, `unavailable`, `unsupported` (the host refused to accept it; not executed); `aborted` (`AbortError`, aborted by `signal`; an accepted one is closed). `openStream` also has `stream` (the producer reported an error, `message` is the host's reason), `device` (audio device not ready), and `timeout` (type `StreamStartError`, `error.stage` is `"prefill"`: the first audio/video buffers were not ready within 10 seconds, or `"activation"`: not activated within 10 seconds after the host selected it; tell them apart by `stage`, not by message text). When `openStream` is aborted by `signal` after the select request has been sent, the SDK first waits for the host's reply to get the source generation, closes only that generation's source, and then releases the producer.
- Errors thrown in handlers called by the SDK (including rejections of async handlers) go to the `onError` of their scope; without a listener they go up to the parent scope, and finally to the plugin log. Plugins do not need `.catch(report)` everywhere.
- Persistent targets such as filters express failure through status: `filter.status` (`waiting`/`applying`/`applied`/`failed`). After `failed` there is no automatic retry until `set()`, `retry()` or a source change.
- The runner has an `unhandledRejection` safety net for plugins (log only), so a forgotten `await` does not crash the plugin process.

## 8. Known limitations

- `media.close()` is conditioned on "still the same source (same generation)" and submits with the latest source stamp: cancelling "the media I opened" closes it even if the user paused in between.
- The `message` of a failed source control is an error code or the backend's text; plugins that need specific wording format it themselves from `code`.
- The runner writes one log line for a request taking more than 10 seconds (without giving up waiting), which makes a stuck host easy to spot.
- Window callbacks must be bound before the first `show()` (the host registers the callback allowlist when it opens the window); panels have no such limit.

The test tools and build tool are in [Test a plugin](https://vestrasys.github.io/vestra-player-sdk/documents/Test_a_plugin.html) and [Command line](https://vestrasys.github.io/vestra-player-sdk/documents/Command_line.html).
