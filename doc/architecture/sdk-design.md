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

Moved to the SDK site, which is generated from the SDK's own types and stays current: the modules and their handles are in the [API reference](https://vestrasys.github.io/vestra-player-sdk/modules/_vestra-player_sdk.html) and [Tutorial](https://vestrasys.github.io/vestra-player-sdk/documents/Tutorial.html); the capability each module needs is in [Capabilities](https://vestrasys.github.io/vestra-player-sdk/documents/Capabilities.html).

Naming choices: user-visible domain nouns (`player`, `filter`, `panel`, `stream`, `snapshot`, `job`, `resolver`); host-internal names (`processing`, `producer`, `source`, `mount`, `$mount`) do not appear in the plugin API. There is no `ctx`: the object a plugin receives is simply called `plugin`, and a feature module receives a `Scope` or a specific handle.

## 3. Lifecycle and release

Activation, declaring as registering, scopes, stopping and what each handle releases: [Tutorial](https://vestrasys.github.io/vestra-player-sdk/documents/Tutorial.html).

## 4. State sync: values, bindings and update passes

`set`, `bind`, update passes, one write in flight per target and name conversion: [Open a tool window and remember its state](https://vestrasys.github.io/vestra-player-sdk/documents/Open_a_tool_window_and_remember_its_state.html). Since SDK 1.5.45 a pass re-runs only the bindings that read the changed state ([large-lists.md §3](large-lists.md#3-p2-precise-invalidation-sdk-only-shipped-in-sdk-1545)).

This is a deliberately small "reactive" system: no virtual DOM and no framework, only "evaluate, compare, send changes"; plugin state is plain variables. The read tracking added in 1.5.45 only decides which bindings a pass re-runs; it does not change what a plugin writes.

## 5. How protocol details are absorbed

_SDK internals, kept here until they move to the SDK repository's `docs/maintaining.md`._

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
| Source | `source.operation(id)` | `pending` means accepted and still running (e.g. URL resolution in progress); keep querying at intervals starting at 40 ms, up to 500 ms, until `completed`/`failed`; if the query itself is rejected by the host → fail with `unknown`. Each plugin can only query its own operations; the host accepts `playback.control` or `playback.read` (`source.operation` in `src/host/src/capabilities/playback_control.rs`) |
| Subtitles | `subtitles.operation(id)` | `completed` → adopt its state; `failed` → failed; `unknown` → fail with `rejected` (not executed); `unknownExpired` → refresh the state and fail with `expired` |
| Snapshots/jobs | `jobs.operation(id)` | `completed` → adopt the result (a snapshot then reads `snapshot.state` again); `failed`/`unknown`/`unknownExpired` → failed; `pending` → query again after 250 ms |
| Package install/uninstall | `packages.operation(requestId)` | The host guarantees `operationId === requestId`; keep querying until a terminal state, never resend |

Under v2 these branches almost never happen in practice (there are no client timeouts), but keeping them means the SDK still gives a definite conclusion when the transport misbehaves, and **never blindly resends a non-idempotent operation** (open, subtitle commit, starting a job, install). Only last-write-wins processing resources can be resent safely, and only when they were definitely not executed.

The root cause of the audio workbench getting stuck on "result of the previous operation awaiting confirmation" (the plugin treated `unknown` as a state needing user confirmation and blocked all updates meanwhile) does not exist in v2: the SDK decides by rule 1 that `unknown` was not executed and handles it, and the plugin only sees `applying` → `applied`/`failed`.

## 6. Explicit conditional submission

Default conditions and `ifCurrent`: [Control playback: Control it, and handle `stale`](https://vestrasys.github.io/vestra-player-sdk/documents/Control_playback.html#control-it-and-handle).

## 7. Error model

Error codes, `onError` and recovery: [Error codes](https://vestrasys.github.io/vestra-player-sdk/documents/Error_codes.html) and [Troubleshooting](https://vestrasys.github.io/vestra-player-sdk/documents/Troubleshooting.html).

The runner's `unhandledRejection` handler only logs, in both runners, so a forgotten `await` crashes neither a plugin's own process nor the shared runner.

## 8. Known limitations

- `media.close()` is conditioned on "still the same source (same generation)" and submits with the latest source stamp: cancelling "the media I opened" closes it even if the user paused in between.
- The `message` of a failed source control is an error code or the backend's text; plugins that need specific wording format it themselves from `code`.
- The runner writes one log line for a request taking more than 10 seconds (without giving up waiting), which makes a stuck host easy to spot.
- Window callbacks must be bound before the first `show()` (the host registers the callback allowlist when it opens the window); panels have no such limit.

The test tools and build tool are in [Test a plugin](https://vestrasys.github.io/vestra-player-sdk/documents/Test_a_plugin.html) and [Command line](https://vestrasys.github.io/vestra-player-sdk/documents/Command_line.html).
