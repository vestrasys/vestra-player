# Shared runner: trusted plugins in one Node process

API maintainer design, 2026-10-27. A plan, not code: nothing in the repositories changed, nothing was built or run (the machine was in use). Host paths are relative to `src/host/src/`, runner paths to `src/runtime/`, SDK paths to the installed package `src/node_modules/@vestra-player/sdk/dist/`. Numbers come from the researcher's memory spike (`memspike/out/report.md` and `summary.json`, 2026-10-26; medians, MB; WS = working set, PB = private bytes). This design composes with [lazy activation](2026-10-lazy-activation.md) (A: record and replay, B1: residency, C: capabilities), which stays as written.

## 0. Decisions in ten lines

1. **Trusted plugins share one Node process and one V8 isolate**: the *shared runner* S. Everything else keeps its own process. First version: built-ins only (user ruling 1). The plugin API, the `plugin` object and the manifest do not change (ruling 3).
2. **Trust is decided by the host from provenance** (ruling 2): built-in = listed in `builtin.json`, running from the installation's own directory, `Instance.builtin` in the catalog, **and** its package digest matches what packaging recorded in `builtin.json` schema 3. No manifest field, no user-facing elevation. A mismatch → own process, with the reason in `diag`.
3. **One pipe, an envelope per message.** S keeps stdin/stdout JSONL; every line carries the owner generation `g`. Per-plugin pipes were compared and rejected (§5.1). The 1 MiB limit stays a per-message limit on the inner message; an oversize line is attributed from its prefix and stops only that plugin.
4. **The runner is split into a boot module and two entries.** `boot.ts` is today's runner body with the process-wide pieces (argv, env, stdio, `process.exit`, console) turned into parameters; `runner.ts` (own process) wraps it exactly as today, `shared-runner.ts` multiplexes N boots. `tests/sdk/runner.test.ts` keeps passing on `runner.ts` unchanged.
5. **No isolation inside S** (ruling 4, VS Code extension-host semantics): one unattributable crash, OOM or hang restarts every plugin in S; the host re-sends nothing itself, plugins re-register by activating again (and, once A lands, their recordings fill the menus meanwhile). A crash loop (3 restarts in 60 s) **fails open to today's topology**: the trusted plugins run in own processes for the rest of the session, with a notice.
6. **Per-plugin attribution inside S** uses `AsyncLocalStorage`: console lines get `[<plugin id>]`, an `uncaughtException` whose async context names a plugin stops that plugin only; one without a context restarts all. Verified in S1 by tests; if attribution proves unreliable, the fallback is restart-all.
7. **Lazy activation composes without changes to its design**: a parked trusted plugin is woken *into* S (started if needed); generations and `rekey` stay per plugin; S itself has a runner id, not a generation. S stops when it holds no live plugin.
8. **Phase 0, both modes, independent of S**: ship `runner.js` instead of `runner.ts` (type stripping loads Amaro's WASM: +14.7 MB WS per process), `NODE_COMPILE_CACHE`, `--max-old-space-size` for S. `UV_THREADPOOL_SIZE` is measured, not set, until §10 says otherwise.
9. **Acceptance**: 18 built-ins headless, second start: 1 node process, node working set ≤ 150 MB (today 1146 MB), node start CPU ≤ 1 s (today ≈ 3 s), every plugin's menu items present, every plugin test and `host-agreement` unchanged.
10. About 5.5 maintainer-days in four steps (P0, S1, S2, S3). Order with the other work: P0 → S1 → lazy C1 → S2 → lazy C2 → lazy A1/B1. Two items need the user's eye under §1.6 (a hung built-in now restarts its siblings; the SDK docs' "its own process" sentence); nothing changes for third-party plugins in v1.

## 1. The problem and the numbers

Today every plugin is a Node process (`plugins.rs:105-153`, `load_manifest`), 18 built-ins ≈ 1146 MB WS / 1277 MB PB idle. What one process costs, from the spike's baselines:

| Layer | WS | PB | Source |
| --- | --- | --- | --- |
| Node 26 idle, no modules | 41.5 | 20.7 | `idle` |
| + libuv threadpool (4 threads, touched by the first fs call) | +0.4 | +32.3 | `threadpool` |
| + SDK runtime (44 modules) | +5.5 | +4.5 | `sdk-runtime` |
| + runner as `.ts` (type stripping: Amaro WASM) | +14.7 | +11.7 | `runner-ts-tiny` vs `runner-js-tiny` |
| + a real built-in's bundle and activation requests | +0.5–2.5 | +0.5–2 | `plugins` table (62.8–65.2 WS) |

So about 63 MB per plugin, of which the plugin's own code is 1–2 MB; the rest is paid 18 times. Three ways to pay it once, measured with 18 built-in activations (`shared.mjs`, `shared-boot.mjs`; totals include the fake-host process):

| Topology | WS idle | PB idle | Start wall | Start CPU | Marginal per plugin |
| --- | --- | --- | --- | --- | --- |
| 18 processes (`proc`, extrapolated from K=8: 559.6 WS) | ≈ 1146 | ≈ 1277 | ≈ 2.8 s | ≈ 3 s (156–188 ms each) | 64.5 WS / 71 PB |
| 18 `worker_threads` in one process | 325 | 434 | 1003 ms | 1055 ms | 15.5 WS / 22.9 PB |
| 18 plugins in **one isolate** | 65.6 | 73.3 | 197 ms | 226 ms | **0.8 WS / 0.7 PB** |

Workers pay a V8 isolate and a heap per plugin; the isolate pays only the plugin's code and state. The SDK's module-level state is per instance (`WeakMap`s keyed by the plugin object; `shared-boot.mjs` activated 18 built-ins in one isolate with no error), so the isolate is possible without SDK changes.

Node flags (`playback`): `NODE_COMPILE_CACHE` 51.8 WS (−13.4) and 200 ms ready (−30); a pre-stripped `runner.js` 51.4 WS; `--max-semi-space-size=1` −2.6; `--optimize-for-size` −1.7; `--jitless`, `--lite-mode` fail to run; a startup snapshot of the bundled SDK is *worse* (51.4 vs 44.3 WS). The compile cache and `runner.js` are the same win (both skip stripping the runner); the cache also caches the plugins' bytecode.

## 2. Goals and non-goals

Goals:

- 18 built-ins cost one Node process; the host, the plugin API, the SDK and the plugins behave the same (ruling 3): the same `createPlugin`, the same requests, the same error codes, the same activation deadline, the same `plugin.dataDirectory`, `plugin.app`, `plugin.i18n.locale`.
- The host alone decides who shares, from provenance it can verify on disk (ruling 2).
- Shared mode is observable (`diag`, the Plugin Center later) and fails open to today's topology.
- The design composes with lazy activation: parked, woken, rekeyed plugins work inside S.

Non-goals:

- Isolation inside S (ruling 4). The capability check per owner stays (it is the host's correctness, not a defence); memory, CPU, `process.*` and crashes are shared.
- Third-party plugins in S. The provenance table (§3.1) reserves the row; the host-side evidence it needs (the signed index's archive digest and key) is not recorded today and is not added in v1.
- Making an own-process runner smaller than Phase 0 does (startup snapshots, `--jitless`: measured, rejected).
- Changing the stop contract (SDK `docs/concepts/plugins.md` "Stopping": 300 ms, then killed) beyond what lazy activation already lists (park waits 2 s).

## 3. The trust decision

### 3.1 Provenance table

The host already distinguishes every origin without reading the manifest for it: `package_store::Source` (`package_store.rs:30-42`: `Repository { url }`, `Link { url }`, `File`, `Workshop`, `Bundled`), `Instance.builtin` (`:66`), `Host.development` for `--plugin` loads (`main.rs:612`), and `is_builtin(owner)` (`package_manager.rs:562-577`: the running plugin's directory equals the `builtin.json` directory *and* the catalog instance is `builtin`). The manifest (`manifest.rs:31-61`) has no trust field and gets none.

| Provenance | How the host knows | Verified before a shared start | Runner |
| --- | --- | --- | --- |
| Built-in | `builtin.json` entry (schema 3), directory `<resources>/plugins/<id>` under the installation (`builtin::check`, `:1541-1569`: a real directory, not a link, inside the root), `Instance { builtin: true, source: Bundled }` | package digest of the directory = `builtin.json` digest (§3.2); checked once per incarnation of S for each plugin it admits | **shared** (v1) |
| Official repository | `Source::Repository { url }` + `StoredPackage.digest` (`package_store.rs:115`, the host's own tree digest from `package_stage::inspect`) | v2: the library copy re-inspected and compared with `StoredPackage.digest`; `url` in a host-side list of trusted repositories; the install record carries the signed index's archive `sha256` and the key that signed it (not recorded today: the store plugin verifies the index, `plugins/store/repos.ts:237,323`, and the archive against it, `installer.ts:16`; the host only receives `source`, `package_manager.rs:1309-1340`) | own in v1; shared by repository policy in v2 |
| Link, file (sideloaded `.vxp`) | `Source::Link`, `Source::File` | — | own, always |
| Workshop | `Source::Workshop` | — | own, always |
| Development `--plugin` | `Host::load` (`main.rs:573`), `Host.development` | — | own, always (also for a reserved `vestra.` id developed in place: `is_builtin` is false because the directory differs) |
| Built-in whose checks fail (`Builtin.manifest: Err`) or whose digest mismatches | `builtin::load` (`:1515-1537`) | — | not started / own, `diag` says why |

`trust(id, directory, instance) -> Trust::{Shared, Own(&'static str)}` lives in `package_manager.rs` beside `is_builtin` and is the single place `load_package` asks (§7.1). `Host::load` never asks: development loads are own by construction.

### 3.2 `builtin.json` schema 3

Written by `scripts/stage-plugins.mjs` (`stagePlugins`, `:103-139`), read by `builtin::read` (`:1471-1510`, today strictly schema 2). Schema 3 adds one field per plugin:

```json
{
  "schema": 3,
  "plugins": [
    { "id": "vestra.playback", "version": "0.2.1", "directory": "vestra.playback",
      "digest": "<64 lowercase hex>" }
  ]
}
```

`digest` is **the package digest the host already computes** for every installed package (`package_stage::read_tree`, `:117-170`): SHA-256 over `DIGEST_DOMAIN = "vestra.package-stage/v1\0"`, then for each file in sorted path order (`/`-joined, the `.vxp` names) `len(path) as u64 LE`, `path`, `len(bytes) as u64 LE`, `bytes`. One digest per package rather than per-file hashes, because:

- the host verifies with one existing function, `package_stage::inspect(&directory)` (`:92-110`), which also re-applies the package limits and manifest checks; a built-in then satisfies exactly the invariant an installed package satisfies (`package_store.rs:1546-1548` uses the same call to check library copies);
- it is the digest `packages.validate` reports (`package_manager.rs:258`) and `StoredPackage.digest` stores, so the v2 third-party check (§3.1) is the same comparison;
- per-file hashes add nothing to the decision (any difference → own process) and would make `builtin.json` 18 × ~40 lines.

`stage-plugins.mjs` computes it in JS over the unpacked directory it just verified against the archive (`unpack`, `:66-101`, already walks every file and holds the bytes): `createHash("sha256")` with the domain, the sorted names and the two little-endian lengths. Two tests pin the two implementations together: a Node test in `src/tests/` hashes a fixture directory with the script's function and compares with a digest committed from `package_stage`'s unit test (`digest_binds_sorted_paths_bytes_and_original_manifest`, `package_stage.rs:340`), and the host test gains the same fixture. A drift fails both sides.

Reading side: `builtin::read` accepts schema 2 **and** 3; `Entry.digest: Option<String>` (64 hex, `digest_name`-shaped). A schema-2 list (an older staging, `VESTRA_BUILTIN_PLUGINS` pointing at a hand-made directory) starts its plugins in own processes with one `diag` line per run: "built-in plugins without digests run in their own processes". After one release schema 2 can be refused; not in this round.

**Cost**: the 17 non-workshop built-ins total ≈ 3.3 MB of `dist/`; the workshop vendors TypeScript and the agent, 28.5 MB. SHA-256 at ~1 GB/s is ~35 ms of CPU for all of them, plus the disk read (cold: a few hundred ms for the workshop). Verification runs on one thread per plugin, overlapped with S's own boot (~140 ms, `idle` ready), and `plugin.start` for a plugin is sent only after its digest passed (§7.1). Under lazy activation the workshop is lazy and is verified at its first wake, not at launch.

**Where it is recorded**: `Builtin` (`:1436-1444`) gains `digest: Option<String>` (from the list) and the per-incarnation result lives in `SharedProcess.verified: BTreeSet<String>` (§7.1); nothing is written to disk. The decision is logged once per plugin per incarnation: `diag!("shared runner: {id} admitted")` / `"… runs alone: {reason}"`.

### 3.3 What is not trust

- `vestra.` ids are reserved (`manifest::reserved_id`, `check_reserved` `:1363-1380`), but the id is not the evidence: a development load of `vestra.playback` from a worktree is `--plugin` provenance and runs alone.
- `Source::Bundled` in the catalog is written by `sync_builtins` from `builtin.json` (`package_store.rs:427`), never by a request (`install_source` refuses it, `:1327-1331`); it is one of the three conditions, not sufficient alone.
- The macOS `.app` signature and the admin-only install directory on Windows (`builtin` module doc, `:1421-1427`) protect the files at rest; the digest check is what the host itself can verify, on every system, including `VESTRA_BUILTIN_PLUGINS` overrides.

## 4. Process topology

### 4.1 Processes

```text
host ──┬── S: shared runner (node, one isolate)  ⟵ built-ins whose digest verified
       │      plugin g=12  vestra.playback
       │      plugin g=13  vestra.playlist
       │      …
       ├── own runner (node)  ⟵ zysilm.youtube   (repository, v1: own)
       ├── own runner (node)  ⟵ a sideloaded .vxp
       └── own runner (node)  ⟵ --plugin <dir>   (development)
```

- **S starts on demand**: the first `load_package` whose `trust` says `Shared` spawns S (if none runs) and queues the plugin. S boots the SDK runtime once; plugins are started as `plugin.start` messages arrive and activate concurrently in one event loop (their `activate` functions interleave at `await`s, as the 18-plugin spike did in 197 ms).
- **S stops when empty**: when its last plugin is stopped (disabled, parked, failed) the host sends `runner.stop` and S exits; the next trusted start spawns a new S. With 8–9 resident built-ins (lazy §3.5) S lives for the session; in a run where everything is parked it costs nothing.
- **One S per host.** Several S (by capability class, by heaviness) are not worth their 42 MB each; a plugin that must not share its fate is a plugin that is not trusted, and runs alone.
- **Start order and the mount gate**: `builtins_starting` / `builtin_mount_expected` (`package_manager.rs:580-628`) work per owner with `activations.status` and `is_builtin`; unchanged. S's boot adds ~150 ms before the first plugin's `activate`, inside the 15 s activation deadline (`load_package`, `:162`) that still starts at `activations.begin`.

### 4.2 Composition with lazy activation

| Lazy concept | In S |
| --- | --- |
| Owner generation (one per process today, `commands.rs:549-551`) | **One per plugin instance**, as today. S has a `RunnerId` (u32) that no registry sees. A plugin stopped and started again in the same S gets a new generation; the stale-event guard (`main.rs:764`) works unchanged. |
| Wake on send (`Host::send` on a parked owner, lazy §2.2) | `load_package` under the new generation → `trust` → S running? send `plugin.start`; else spawn S first. The queued trigger is forwarded at `runtime.ready` as designed. Cold wake of a built-in into a running S: no node boot, ≈ 50–100 ms to its first registration (`isolate-1`: 50 ms wall), well under the 1 s p95 of lazy §5.2. |
| `rekey(from, to)` | Per plugin, unchanged: registries move the parked generation's entries to the new one; S learns the new generation in `plugin.start`. |
| Park (stop, keep registrations, lazy §2.3) | `plugin.stop { g }` to S; the plugin's `dispose()` runs inside S; the host waits for `runner.stopped { g }` up to 2 s (the park wait lazy §2.3 introduces) and then counts a leak (§8.4). S keeps running for the others. |
| `residentAtReady`, recordings | Unchanged: they are per owner, computed from the registries. |
| Replay at start | Unchanged: parked owners have no process at all; S is spawned by the first wake or by the resident plugins at launch. |
| First-run throttle (8 processes) | Trusted first runs do not count against it (they are no processes); the throttle counts own-process runners. |
| `VESTRA_LAZY=0` | Orthogonal: forces everything live, still in S when trusted. `VESTRA_SHARED=0` (new, §7.6) forces own processes, still lazy. |

Order of landing matters for one thing: A1's wake path calls `load_package`, which by then routes through `trust`. If S2 lands first (recommended, §12), A1 needs no S-specific code.

## 5. Wire protocol

### 5.1 One pipe with an envelope, or one pipe per plugin

| | (A) one stdin/stdout pair, envelope `g` per line | (B) one pipe per plugin (fds 3…N, named pipes / Unix sockets, or `MessagePort`) |
| --- | --- | --- |
| Host threads | 1 reader + 1 writer for S | 2 per plugin (as today), or a poller |
| Ordering | FIFO per plugin and across plugins (the same single-pipe guarantee as today, per plugin) | FIFO per plugin only |
| Backpressure | one OS pipe; per-plugin accounting in the host (§5.3) | per plugin by the OS |
| Attributing a fault | from the envelope prefix (§5.4) | by pipe |
| Node side | `process.stdout` / `stdin`, as today | extra fds: `child_process` with `stdio: [..., 'pipe', ...]` is the host's side in Rust — `std::process::Command` has no portable extra-fd stdio; Windows needs named pipes; `MessagePort` exists only between threads |
| Per-plugin wake into a running S | one message | create a pipe, pass it to a running process: not possible after spawn without a handle-passing protocol |
| Protocol docs and tests | the inner message is unchanged; `runner.test.ts` unaffected; one envelope type | a new transport per OS |

(A). The last two rows decide it: (B) cannot add a pipe to a running process portably, so a parked plugin could not be woken into S without restarting S.

### 5.2 Messages

Every line S writes or reads is one JSON object. **The envelope is the inner message plus `g`** (no nesting, so the host's `Request` parsing and the runner's reply dispatch stay byte-compatible after dropping one key):

```jsonc
// plugin → host (today's shape + g; g is the first key, see §5.4)
{"g":12,"id":7,"method":"commands.register","params":{...}}
{"g":12,"event":"runtime.ready"}
{"g":12,"event":"runtime.failed","data":{"code":"activationFailed","message":"..."}}
// host → plugin
{"g":12,"id":7,"result":{...}}
{"g":12,"event":"source.state","data":{...}}
{"g":12,"event":"runtime.stop","data":null}            // unchanged stop event, per plugin
// runner control, g = 0
{"g":0,"event":"runner.hello","data":{"pid":1234,"node":"26.1.0"}}            // S → host, once
{"g":0,"event":"plugin.start","data":{"g":12,"id":"vestra.playback","directory":"…","dataDirectory":"…","locale":"zh-Hans","app":{...}}}
                                                                                 // stopping one plugin is today's runtime.stop with its g (above); no separate control message
{"g":0,"event":"runner.stopped","data":{"g":12,"timedOut":false}}             // S → host after dispose settled or 5 s
{"g":0,"event":"runner.fault","data":{"g":12,"message":"…"}}                  // S → host: this plugin is gone (uncaught exception attributed to it)
{"g":0,"event":"runner.ping","data":{"seq":41}}  /  {"g":0,"event":"runner.pong","data":{"seq":41,"stats":{...}}}
{"g":0,"event":"runner.stop"}                                                   // host → S: stop every plugin, exit
```

`plugin.start` carries what `load_manifest` passes through the environment today (`VESTRA_PLUGIN_DATA_DIR`, `VESTRA_APP_INFO`, `VESTRA_LOCALE`, `plugins.rs:138-145`) and what the runner reads from `argv[2]` (`runner.ts:26`). `VESTRA_FFMPEG` / `VESTRA_FFPROBE` stay process environment (one value for all, read by the SDK's `local.js:119`), as does `VESTRA_TRACE`.

The own-process runner's wire does not change at all (no `g`): `runner.ts` keeps today's lines, so `tests/sdk/runner.test.ts` and the `host-agreement` test (`MESSAGE_LIMIT` vs `MAX_MESSAGE_BYTES`, `host-agreement.test.ts:782-816`) are untouched.

### 5.3 Ordering and backpressure

- **Ordering**: per plugin, exactly today's guarantee (one FIFO pipe each way). Across plugins S adds FIFO too; nothing depends on it.
- **Host → S**: today each plugin has a `sync_channel(1024)` and a stall disables that plugin (`OUTBOUND_CAPACITY`, `plugins.rs:46-48`, `send` `:225-230`). S gets one writer thread and one channel of capacity `1024 × plugins.len().max(1)`, with a **per-plugin in-flight count** in `SharedProcess` (incremented at `send`, decremented when the writer thread has written the line). A plugin whose count exceeds 1024 gets today's fault (`plugin output backpressure`) and is stopped in S; the others continue. A full channel (every plugin stalled: the event loop itself is stuck) is the heartbeat's case (§8.3).
- **S → host**: one reader thread; `Event::Request(g, inner)` per line as today (`main.rs:763`); control lines become `Event::Runner(RunnerId, Control)`.

### 5.4 `MAX_LINE` per plugin

The limit stays **1 MiB per inner message, plugin-visible as today**: the runner refuses an oversize request before sending it with `limit` (`runner.ts:69-76`), the SDK's `MESSAGE_LIMIT` (`capabilities.js:267`) and the host's `MAX_MESSAGE_BYTES` (`plugins.rs:19`) stay equal, the agreement test keeps checking them. The envelope adds `"g":<digits>,` (≤ 24 bytes); the host reads S's lines up to `MAX_MESSAGE_BYTES + 64`.

An oversize line from S: the reader has the first bytes, and `g` is the first key by construction (S's `send` builds `{"g":…` itself; plugin code never writes the pipe, §9.2), so the host attributes the line from its prefix, discards the rest of the line (`read_until` in a loop until the newline), sends `Event::RuntimeFault(g, "protocol message exceeds 1 MiB")` for **that plugin**, and keeps reading. Today the reader breaks out and the plugin dies (`:185-191`); in S the plugin dies and S lives. A line with no parsable `g` prefix or invalid JSON is a **runner** fault (nothing else can have written it) → restart-all (§8.2).

## 6. Runner changes (`src/runtime/`)

### 6.1 Layout

| File | Role |
| --- | --- |
| `boot.ts` | today's `runner.ts:26-207` as `boot(options): Promise<Instance>`: reads the manifest, builds `request`/`on`/`stop`, calls `createPlugin`, imports the entry, runs `activate`, waits `activated()` with the `TAKEN` rule, posts `runtime.ready` / `runtime.failed`. Options: `directory, dataDirectory, locale, app, post(message), log(...args), trace`. Returns `{ stop(): Promise<void>, deliver(message) }`. Has **no** reference to `process.argv`, `process.env`, `process.stdout`, `process.exit`, `console`. The spike's `shared-boot.mjs:13-60` is this function. |
| `runner.ts` | own process: today's lines 1-25 (stderr/stdout error handlers, console redirection), then `boot({ directory: argv[2], dataDirectory: env…, post: line → stdout, log: console.error })`, `stdin` lines → `deliver`, `runtime.stop` → `stop()` → `process.exit(0)`, `runtime.failed` → `process.exit(1)`. Behaviour identical; `runner.test.ts` is the proof. |
| `shared-runner.ts` | S: `Map<g, Instance>`; stdin lines → by `g`: 0 → control, else `instances.get(g)?.deliver(inner)`; `post` adds `g`; `plugin.start` → `boot(...)` inside `als.run(g, …)`; `runtime.stop` for `g` → `instance.stop()` → `runner.stopped`; `runner.stop` → stop all concurrently, `process.exit(0)` after they settle or 5 s; `runner.ping` → `runner.pong` with stats. |

`stage-runtime.mjs` ships the three (as `.js`, Phase 0 §10.1); `runtime_paths.rs` adds `shared_runner: PathBuf` next to `runner`.

### 6.2 Process-wide concerns, per plugin

| Today (`runner.ts`) | In S |
| --- | --- |
| `process.exit(0)` after `stop` (`:122`), `process.exit(1)` after `runtime.failed` (`:217`), `setTimeout(exit, 5000)` guard (`:116`) | none per plugin. `stop()` disposes and resolves; a `dispose()` that has not settled after `STOP_TIMEOUT_MS` (5 s) is abandoned: `runner.stopped { g, timedOut: true }`, the instance is dropped from the map, its pending requests were already rejected. A plugin whose `activate` throws gets `runtime.failed` and is disposed; nothing else happens to S. |
| `console.log/info/debug → console.error` (`:23-25`) global | the same override, once, plus a prefix `[<plugin id>] ` from `als.getStore()`, `[shared] ` when none. Still stderr, still inherited (`plugins.rs:151`), still best effort (`:11-15`). |
| `process.on("unhandledRejection", log)` (`:183-185`) | one handler; the line carries the plugin prefix when the store is available in the handler (promise continuations carry the `AsyncLocalStorage` store; whether the `unhandledRejection` event does is what S1's test settles), else `[shared] unhandled rejection`. Never fatal, as today. |
| an uncaught exception kills the process → `Event::Exited` → that plugin is `failed` | `process.on("uncaughtException", e)`: `als.getStore()` names a plugin → `runner.fault { g }`, the instance is disposed (best effort) and removed; no store → `runner.fault { g: 0 }` and `process.exit(1)` → the host restarts all (§8.2). S1 tests both: a throw inside a timer set during `activate` (store expected), a throw from a `process.nextTick` queued by a bare `import()`-time statement (no store expected). |
| `process.stdout.on("error")` → exit or stop (`:19`) | S: a broken stdout means the host is gone: stop all, exit. |
| `input.on("close")` → stop (`:164-166`) | S: stdin closed → `runner.stop` semantics. |
| `process.argv[2]`, `VESTRA_PLUGIN_DATA_DIR`, `VESTRA_APP_INFO`, `VESTRA_LOCALE` | fields of `plugin.start`. The locale is one value per run anyway (`i18n::locale()`); it travels per plugin so the runner has no global. |
| `nextId`, `pending`, `listeners`, `stopping` module globals (`:31-40, 105`) | fields of `Instance`; request ids are per plugin (the host's `Reply { owner, id }` is already per owner). |
| `process.stdout.write` by plugin code corrupts its own channel | S replaces `process.stdout.write` with a function that forwards to stderr (with the prefix) and keeps the original bound privately for its own `post`. See §9.2 for the limits of this. |

### 6.3 Per-plugin accounting (cheap part only)

`runner.pong.stats`: `{ heapUsed, heapTotal, rss, external }` for the process (`process.memoryUsage()`), and per `g`: `{ requests, events, pending, handlerMs }`, where `handlerMs` is `performance.now()` deltas around each `deliver` (replies and events dispatched into the plugin) and around `activate`. It attributes synchronous work in handlers, not continuations; good enough to point at the plugin that burns CPU. Per-plugin heap is **not** attempted (it needs heap snapshots). The host logs the stats on each pong at `VESTRA_TRACE`, and prints the top plugin by `handlerMs` in the hang notice (§8.3).

### 6.4 Tests (S1)

`src/tests/sdk/shared-runner.test.ts`, the same shape as `runner.test.ts` (spawn the runner, answer like the host):

- two plugins start, both reach `runtime.ready`, requests carry their own `g`, replies with the wrong `g` are ignored;
- one `activate` throws: `runtime.failed` for it, `runtime.ready` for the other, S alive;
- `runtime.stop` for one: `runner.stopped { g }`, the other still answers events; `runner.stop`: both stopped, exit 0;
- a `dispose` that never settles: `runner.stopped { timedOut: true }` after 5 s (shortened by an env knob for the test);
- console lines carry the prefix; a 1 MiB + 1 request is refused with `limit` and never written;
- the two `uncaughtException` cases of §6.2;
- `process.stdout.write("junk")` from a plugin reaches stderr, not stdout.

## 7. Host changes

### 7.1 `plugins.rs` → the process layer

`Plugin` (`plugins.rs:84-97`) keeps the per-owner state (`manifest, directory, catalog, data_key, data_directory, subscriptions`) and loses `child` and `outbound`:

```rust
pub enum Runner {
    Own(OwnProcess),                                   // today's Child + SyncSender + the two threads
    Shared { id: RunnerId, g: u64, outbound: SyncSender<Value>, in_flight: Arc<AtomicUsize> },
}
/// Lives in `Host.shared`, not in `Plugin` (a `Plugin` is dropped on a reaper thread, `main.rs:725-731`).
pub struct SharedProcess {
    id: RunnerId, child: Child, outbound: SyncSender<Value>,
    members: BTreeMap<u64 /* g */, Membership { in_flight: Arc<AtomicUsize>, started_at: u64 }>,
    awaiting_stop: BTreeMap<u64 /* g */, u64 /* deadline ms */>,
    verified: BTreeSet<String /* plugin id */>,
    last_pong: u64, missed: u8, leaked: u8, restarts: VecDeque<u64>,
}
impl Plugin {
    pub fn send(&mut self, message: Value) -> Result<()>  // Own: as today; Shared: add "g", bump in_flight, try_send
}
```

- `load_manifest` gains a `Runner` parameter; the host decides before calling it: `trust(...)` (§3.1) → `Own` → spawn as today; `Shared` → `shared.get_or_spawn()`, verify the digest (thread; the plugin is `Pending` meanwhile), then `plugin.start`.
- `Drop for Plugin` (`:255-275`: `runtime.stop`, 300 ms, kill) becomes `Runner::stop(owner)`: `Own` as today, on the reaper thread the host already spawns in `disable()` (`main.rs:725-731`); `Shared` needs no thread: it sends `runtime.stop` with `g` and files `awaiting_stop[g] = now + 300 ms` (2 s for a park); `Event::Runner(runner.stopped { g })` or the tick passing the deadline produces `Event::Reaped(owner)` as today, with `leaked += 1` when the deadline won.
- Reader thread for S: envelope → `Event::Request(g, inner)` / `Event::Runner(id, control)`; EOF → `Event::RunnerExited(id)`; oversize per §5.4.
- Heartbeat: `runner.ping` every 2 s from the existing tick (`activation_tick` runs on the loop, `main.rs:5289, 5400`), only while S has members; `missed ≥ 5` (≥ 10 s) → §8.3.
- Node flags for S: `--max-old-space-size=1024` (§8.4); own runners keep Node's default.

The lazy design's C2 moves this file to `process/process.rs`; §12 orders S2 before C2 so the move carries the new code.

### 7.2 `main.rs`

- `Event::{Runner(RunnerId, Control), RunnerExited(RunnerId), RunnerStopped(u64)}`; `Host.shared: Option<Rc<RefCell<SharedProcess>>>`.
- `disable(owner)` (`:628-732`) is unchanged for the registries; its last step uses `Runner::stop`.
- `handle(Event::RunnerExited | fault g=0 | heartbeat)` → `restart_shared(reason)` (§8.2).
- `send` / `broadcast` (`:733-760`) unchanged in shape: a `send` error on a shared member disables that member only (per-plugin in-flight overflow).

### 7.3 `package_manager.rs`

- `trust()` beside `is_builtin`; `Builtin.digest`; `builtin::read` schema 2|3.
- `start_installed` (`:450-494`) unchanged except that `load_package` now routes; the restart path (§8.2) reuses it with `only: Some(id)` per affected plugin.
- `reload` (`:882-918`) of a built-in: stop in S, drop its id from `verified`, start again (re-verifies).
- `packages.list`: no new field in v1 (§13); `diag` carries the mode.

### 7.4 Stop and timeout semantics, side by side

| Case | Today | Own runner (unchanged) | Shared member |
| --- | --- | --- | --- |
| disable, uninstall, upgrade | `runtime.stop`, 300 ms, kill | same | `runtime.stop { g }`, 300 ms for `runner.stopped`, else counted as leaked; S lives |
| park (lazy) | — | 2 s then kill | 2 s then leaked |
| quit | each `Plugin` dropped: 300 ms each | same | one `runner.stop` to S; S disposes all concurrently (5 s cap inside); host waits 300 ms then kills S — the documented contract, now shared by all members |
| activation timeout (15 s) | `disable(owner)` | same | same; the member is stopped in S |
| `runtime.failed` | process exits 1 | same | member disposed by S, `Event::Request(g, runtime.failed)` as today; S lives |

### 7.5 Crash of S → restart all, with notice

See §8.2. The notice is an OSD (`show_osd`, `main.rs:3116`), one line: "Built-in plugins restarted after a fault" with the attributed plugin's name when known; the `diag` line has the reason and the stats of the last pong.

### 7.6 Switches

`VESTRA_SHARED=0` runs every plugin in its own process (tooling, bisecting, the old GUI scripts that count node processes). `VESTRA_SHARED_PING_MS` (default 2000) for tests. Both are host-internal environment variables like `VESTRA_LAZY`.

## 8. Failure modes

### 8.1 One plugin throws in `activate`

`boot` catches it (today's `runner.ts:208-217`): `runtime.failed { g }`, the SDK instance is disposed, the host runs `activation_tick` → `record_failure` + `disable(owner)` as today. No other member notices. Same for a rejected `activated()` (a registration refused by the host).

### 8.2 S exits, OOMs, or faults unattributed → restart all

Trigger: `Event::RunnerExited`, `runner.fault { g: 0 }`, a line the reader cannot attribute, or the heartbeat (§8.3).

1. `restarts.push(now)`; if three within 60 s → **fail open**: `shared = None`, `shared_disabled_until_quit = true`, every member is restarted in its own process by `start_installed(only)`, OSD "Built-in plugins run separately for this session after repeated faults", `diag` with the three reasons. Today's topology is the floor.
2. Otherwise, for each member `owner`: `record_failure(owner, "shared runner exited: <reason>")`, `disable(owner)` (registries revoked as for any exit; menus of the members vanish for the restart), the backend gets `owner.disabled` as today.
3. Kill the old child if still alive; spawn a new S; `start_installed(only: Some(id))` for each affected id in `builtin.json` order (the catalog's order), each verified again (new incarnation, empty `verified`). Plugins re-register by activating: nothing is replayed by the host itself in v1. With lazy A landed, the recordings of the affected plugins are replayed first and the plugins woken, so the menus are back before the plugins are.
4. If the fault was attributed (`runner.fault { g }` preceded the exit, or the pong stats name a plugin by `handlerMs` in a hang), that plugin is **quarantined for the session**: `trust` returns `Own("faulted in shared runner")` for its id, so the second start is in its own process while the others return to S. Attribution is a hint, not evidence; the quarantine is a stability measure, not a trust change.

User-visible: the members' windows close and their panels unmount for about a second (S boot ≈ 150 ms + activations), playback continues (media the plugins opened keeps playing, SDK docs "Stopping"), an OSD line. This is the VS Code extension-host behaviour the user accepted (ruling 4).

### 8.3 Infinite loop or long synchronous work → heartbeat

S answers `runner.ping` from its event loop; a member spinning synchronously blocks every member and the pong. After 5 missed pings (≥ 10 s; the same order as the runner's "still waiting after 10 seconds" log, `runner.ts:85`, and well above the longest legitimate synchronous work seen in built-ins: Slint-free JSON parsing, the NES core's per-frame step) the host treats S as hung: §8.2 with reason "no heartbeat for 10 s", top `handlerMs` plugin from the last pong named. Own-process plugins have **no** heartbeat today and get none here (§13 item 2 discusses whether they should).

Why not isolate CPU with a worker per plugin instead: 325 MB for 18 (§1); the point of S is the isolate.

### 8.4 Memory growth

- S runs with `--max-old-space-size=1024`: a leaking member takes S down at 1 GB instead of the machine; that is §8.2 (OOM is an exit; V8 prints the fatal error on stderr; attribution comes from the last pong's `heapUsed` trend only if a member's `handlerMs` also stands out — otherwise unattributed).
- `heapUsed` in the pong is logged; at 512 MB the host logs a warning once per incarnation. No action on memory alone in v1.
- Leaked members (`runner.stopped { timedOut: true }`): a member whose `dispose` never settled may hold timers and memory. At `leaked ≥ 3` the host restarts S at the next quiet moment (no `busy` command, no dialog, no pending activation) with the §8.2 path and reason "leaked members".

### 8.5 S fails to spawn

`node_failure()` as today, per plugin: each trusted plugin is recorded `failed` with the Node reason; nothing else differs from today's per-plugin spawn failure.

### 8.6 Digest mismatch

`trust` → `Own("digest mismatch")`: the plugin runs, alone, and `diag` says so once. It is not a failure to the user (the files may be a legitimately patched local build); it is a reason not to share.

## 9. Security notes

### 9.1 What S exposes between its members

Everything a process shares: `process.env` (read **and** write), `process.cwd()` / `chdir`, `process.exit`, `process.title`, `globalThis`, the `node:` built-in module instances, the one SDK copy, timers and the event loop, the heap, file descriptors, `process.pid`. Plugin modules themselves are distinct module instances (distinct `file:` URLs under distinct directories), so module-level state does not collide; a vendored copy of the same npm package in two built-ins is two instances too (two paths).

This is **by design** (ruling 4): the members are the player's own code, shipped and verified together. The host still checks every request's capability against *that owner's* manifest (`plugin.allows`, `main.rs:3579-3587`): a member cannot use a sibling's capability through the protocol, because the envelope `g` is written by S's `post`, which plugin code cannot call. It *could* write fd 1 directly (`fs.writeSync(1, …)`) and forge a line; §9.2 makes the accidental case harmless and the deliberate case is not in the threat model for built-ins.

Audit of the 18 built-ins (`src/plugins/**/*.ts`, excluding `vendor/`, `dist/`, `tests/`): no `process.exit`, `process.chdir`, `process.stdout` or `process.on` in plugin code (the workshop's `worker/worker.ts` has them, but it is the entry of a *child* process, §9.4).

### 9.2 stdout

S replaces `process.stdout.write` so that a stray `console.log` from a vendored library (already redirected today, `runner.ts:23`) or a raw write goes to stderr with the plugin prefix; `post` keeps the original. `fs.writeSync(1)` and `process.stdout._handle` remain reachable; a line that does not start with `{"g":` is treated as a runner fault (§5.4) and restarts S rather than being misattributed. The own-process runner gets the same `process.stdout.write` redirection (today such a write breaks that plugin's own channel; afterwards it is logged) — a hardening, listed in §13.

### 9.3 `process.pid` in temporary names

Found (non-vendor, non-test): `visualizer/logo.ts:88`, `visualizer/covers.ts:351,380` (`<target>.<pid>.tmp`), `hotkeys/keys-page.ts:495`, `playlist/listfile.ts:112` (`<path>.tmp-<pid>`), `info/dialog.ts:164` (`<chosen>.vestra-<pid>.tmp`), `capture/copy.ts:16` (`tmpdir()/vestra-capture-<pid>`); in the SDK `plugin/services.js:114` (`<path>.tmp-<pid>-<counter>`) and `plugin/markdown.js:139` (`<file>.<pid>.tmp`). With one pid for 18 plugins these names collide **only** when two members write a temporary for the *same target path* at the same time. Every target above is inside the writing plugin's own data directory or is a user-chosen path (`info/dialog.ts`, `hotkeys` export), except `capture`'s `tmpdir()` directory, whose prefix no other plugin uses. No collision exists today; the risk is a future one. Recommendation to plugin authors (SDK docs "Write atomically", and a one-line note in each plugin's next round): use `plugin.dataDirectory` and `crypto.randomUUID()` (or the SDK's `storage.json`, which already has a counter) instead of `process.pid`. Not a blocker, not maintainer work.

### 9.4 Child processes

The workshop spawns Node children with `--permission` (`workshop/agent.ts:71`, `run.ts:41`, `toolchain.ts:331,469`) using `process.execPath`: the same bundled Node whether the parent is S or an own runner. A restart of S orphans a running child exactly as a kill of the plugin's own process does today (Windows does not kill children with the parent); the plugin's `onDispose` is its chance to stop them, in both modes. Unchanged.

### 9.5 Secrets and data directories

`DataKey::secret_service` and `plugin-data/<GUID>` (`plugins.rs:22-45`) are host-side, per owner; S receives each member's `dataDirectory` in `plugin.start` and nothing of the others'. A member can of course read a sibling's directory through `fs` — it can today too (the path layout is "not part of the API", SDK docs); the built-ins do not.

## 10. Phase 0: cheap wins for both modes

Independent of S; land first and record the new baseline (the lazy design's P0 numbers move with them).

### 10.1 `runner.js` instead of `runner.ts`

`stage-runtime.mjs` copies `runner.ts` (`:38`) and the host runs it with Node's type stripping; the Amaro WASM strip costs **14.7 MB WS / 11.7 MB PB per process** and ~40 ms (`runner-ts-tiny` vs `runner-js-tiny`). Staging emits `runner.js` (and `boot.js`, `shared-runner.js`) with `module.stripTypeScriptTypes` (Node ≥ 22.13, present in 26) or `tsc`; `runtime_paths.rs` points the packaged layout at `.js` and the development build at `src/runtime/*.ts` (as today, no build step for developers). `runner.test.ts`'s packaged-layout test (`:65-93`) runs the staged `.js`. For 18 own processes: −265 MB; for S: once.

### 10.2 `NODE_COMPILE_CACHE`

`node_command` (`runtime_paths.rs:76-92`) sets `NODE_COMPILE_CACHE=<app data>/node-compile-cache` (per user, writable; `platform::app_data_dir`). Measured −13.4 MB WS and −30 ms ready for `playback` with the `.ts` runner; with `runner.js` most of that is already taken, what remains is the plugins' and SDK's bytecode (~0.8 MB bytecode per process in the spike's `bytecode` column, and the compile CPU). Expected −1–3 MB and −20 ms per process; measured in P0, kept if ≥ 10 ms.

### 10.3 `--max-old-space-size` for S

1024 MB (§8.4). Own runners: no change (a per-plugin cap would be a behaviour change for third-party plugins).

### 10.4 `UV_THREADPOOL_SIZE`

The threadpool costs 32 MB **PB** per process but only 0.4 MB WS (`threadpool` vs `idle`): four thread stacks reserved, not touched. Setting `1` saves commit charge, not resident memory, and serialises fs, zlib and crypto work (the store unzips, subtitle-online inflates, capture copies). Not set in P0; recorded as a knob (`VESTRA_UV_THREADS`) to try on a commit-limited machine. With S the question is moot for built-ins (one pool).

### 10.5 Not done

Startup snapshot (worse), `--jitless` / `--lite-mode` (fail to run), `--optimize-for-size` and semi-space (≤ 2.6 MB, startup CPU cost).

## 11. Acceptance

Method: the lazy design's (§5.2 there): installed layout (`VESTRA_BUILTIN_PLUGINS`), `vestra.exe --headless --silent --seconds 20`, at t = 15 s `Get-Process node` count, `WorkingSet64` sum, `TotalProcessorTime` sum; median of three. Profiles: shared on (default), `VESTRA_SHARED=0` (today).

| Scenario | Today (spike, 18 built-ins) | Accept when |
| --- | --- | --- |
| Phase 0 alone, 18 own processes | 18 processes, 1146 MB WS, ≈ 3 s node CPU | 18 processes; node WS ≤ 900 MB; ready time per plugin ≤ 190 ms median |
| S, 18 built-ins, second start, lazy off | — | **1** node process; node WS ≤ 150 MB (spike isolate: 65.6 incl. a fake host; real activations add their state: estimate 90–120); node start CPU ≤ 1 s; every plugin `running` in `packages.list`; every menu item of every plugin present; time to the last `runtime.ready` ≤ today's |
| S + lazy (A1/B1 landed), second start | — | 1 node process holding the 8–9 resident built-ins; node WS ≤ 110 MB |
| Mixed: S + one `--plugin` dev load + one sideloaded `.vxp` | — | 3 node processes; the two own ones behave as today (same tests) |
| Digest mismatch (one built-in file edited under `VESTRA_BUILTIN_PLUGINS`) | — | that plugin alone in its own process, `diag` names it; the rest in S |
| One member throws in a timer after ready | — | that plugin `failed`, the others unaffected (headless test with a fixture built-in list) |
| One member spins (`while(true)`) | — | within 12 s all members restarted once, OSD shown, a second start in S for the others and own process for the spinner |
| Three faults in 60 s | — | fail-open: 18 own processes, notice, no further restarts |
| No plugin-visible change | — | `npm run plugin -- test <name>` for all 18 unchanged and green; `runner.test.ts` unchanged and green; `host-agreement.test.ts` unchanged and green; the SDK TestHost untouched |

## 12. Phases, estimates, order

| Phase | Content | Days | Gate |
| --- | --- | --- | --- |
| **P0** | §10.1–10.3: `runner.js` staging, `NODE_COMPILE_CACHE`, `--max-old-space-size` plumbing, `runtime_paths.rs`; measurements before/after | 1 | numbers recorded; `runner.test.ts` green on the staged `.js` |
| **S1** | `src/runtime/boot.ts`, `runner.ts` (thin), `shared-runner.ts`; `shared-runner.test.ts` (§6.4); `stage-runtime.mjs` ships all three. **Touches only `src/runtime/`, `src/scripts/stage-runtime.mjs`, `src/tests/sdk/`** — no host conflict with anything in flight | 1.5 | tests green; the own runner's wire byte-identical (`runner.test.ts` untouched) |
| **S2** | host: `Runner` enum, `SharedProcess`, envelope reader/writer, heartbeat, `Event::Runner*`, stop semantics, `trust()`, `builtin.json` schema 3 in `stage-plugins.mjs` + `builtin::read` + digest cross-test, restart-all with loop guard and fail-open, quarantine hint, OSD, `VESTRA_SHARED`; host unit tests (envelope parsing, oversize attribution, trust table, loop guard) | 2 | all §6.1 checks; headless fixture tests of §11 rows 5–8 |
| **S3** | measurements of §11 on this machine; coordinator GUI pass (§6.2 of the workflow) with S on | 1 | §11 numbers |

≈ 5.5 maintainer-days plus review and GUI.

**Order relative to the other branches** (worktrees as of today: `gpu-render`, `ui-fixes`, `lazy-activation-design`, `store-finish` and the rest are at or behind `main` `eae5f48`; the GPU *gate* (#51) is merged, the lazy design is an uncommitted document):

1. **P0** — any time; only `stage-runtime.mjs`, `runtime_paths.rs`, `runner.test.ts`. Before the lazy P0 baseline, so the baseline is measured once.
2. **S1** — any time, in parallel with GPU and UI work (runtime files only).
3. **Lazy C1** (capabilities trait; does not touch `plugins.rs`, `Host`, `main()`) — in parallel with S1/S2.
4. **S2** — before **lazy C2**: C2 `git mv`s `plugins.rs` → `process/process.rs` and splits `main.rs`; landing S2 first means the move carries S2's code and no hunk is rebased twice. S2's `main.rs` edits are in `disable`/`handle`/`send`/`Event` (`:247-268, 628-760, 1093-1119`), away from the GPU hunks (`@473, @1319, @1402, @5138, @5258`) and from UI files.
5. **Lazy C2**, then **A1/B1**: A1's wake path calls `load_package`, which routes through `trust` by then; A1 adds no S-specific code. B1's `packages.list` `process: "running" | "idle"` field is the natural place for a later `"shared"` detail (§13).

If S2 must wait for a GPU render implementation branch (none exists today beyond the gate), S1 and P0 still land.

## 13. §1.6 classification

**Additions (maintainer decides, no plugin-visible change)**: the runner split and `shared-runner`; `builtin.json` schema 3 (packaging; the reader accepts 2 and 3); the envelope protocol between host and S (the own runner's wire is unchanged; no SDK change: the SDK never sees the envelope); `Runner`/`SharedProcess`/`trust`; heartbeat, restart, loop guard, quarantine hint, fail-open; `VESTRA_SHARED`, `VESTRA_SHARED_PING_MS`; `runner.js` staging; `NODE_COMPILE_CACHE`; `--max-old-space-size` for S; `runner.pong` stats in `diag`. `PluginInfo` gains nothing in v1 (a `process` detail is deferred to lazy B1's field).

**Behaviour changes to put before the user (§1.6 item 2), all limited to built-ins in v1:**

1. **Shared fate** (the user's own ruling 4, recorded here for the §1.6 trail): an unattributable crash, an OOM or a hang of one built-in restarts every built-in in S; their windows close and panels unmount for about a second; `activate` runs again mid-session. Third-party plugins (own process) are unaffected. Lazy §5.4 item 1 already asks for "activate may run more than once per session".
2. **Doubtful: a hung built-in now restarts its siblings after 10 s.** Today a built-in spinning forever stays hung until the user disables it (no detection); with S the heartbeat detects it and §8.2 runs. Strictly this is a change in *when something happens* for the hung plugin's siblings. Alternative if not wanted: detect and log only, no restart (then a hung member hangs all of S until the user acts, which is worse for the user; the maintainer recommends the restart).
3. **Doubtful, minor: `process.stdout.write` from plugin code is redirected to stderr** in both runners (today it corrupts the plugin's own channel and stops it). No built-in does it; a third-party plugin that did would have been failing already.
4. **SDK docs**: `docs/tutorials/getting-started.md:12` says "Each plugin runs in its own Node.js process". It becomes "its own Node.js process; the player's built-in plugins share one" and `docs/concepts/plugins.md` gains a paragraph on what that shares (`process.*`, `pid`, memory). A docs patch in the SDK repository, released with S2. No third-party plugin's process model changes.
5. **Quit**: the 300 ms window is shared by all members' disposals (today 300 ms each, in parallel threads). Same documented contract; mentioned for completeness.

Nothing in this design changes an error code, a default, a signature, or when a third-party plugin's work happens.

## 14. Open questions for the user

1. §13 item 2: accept that a hung built-in is detected after 10 s and restarts all built-ins (recommended), or detection with a log only?
2. §8.2 step 1: accept the fail-open after three restarts in 60 s (the rest of the session in own processes, with a notice)? The alternative is to keep retrying S, which can loop on a bad build.
3. For v2 (third-party in S): should the host itself verify the repository index signature (a Rust Ed25519 dependency, `cargo-deny` review) rather than trust the store plugin's verification and recorded evidence? No work now; it decides what the install record must carry.
4. Phase 0 before or after the lazy design's baseline measurement (recommended: before, one baseline for both).

### User ruling (2026-10-27)

The user approved starting the work ("可以开工") with the coordinator's recommended answers to §14: (1) a hung built-in (no heartbeat for 10 s) restarts all built-ins in the shared runner; (2) three restarts within 60 s fall back to one process per plugin until exit, with a notice; (3) install records start storing the repository index signature evidence now, host-side signature verification is for v2; (4) phase 0 (precompiled `runner.js`, `NODE_COMPILE_CACHE`) lands before the lazy-activation baseline. Trust is decided by the host from provenance, never from a manifest field; the plugin API does not change; v1 admits built-ins only.

## 15. Implementation status

| Phase | Status (2026-10-27, branch `shared-runner`) |
| --- | --- |
| P0 | Done: packaged `runner.js`; `NODE_COMPILE_CACHE` measured and **not kept** (§15.1); `--max-old-space-size` moves to S2 (only the host's spawn of S uses it) |
| S1 | Done: `boot.ts`, thin `runner.ts`, `shared-runner.ts`, `tests/sdk/shared-runner.test.ts`; nothing in the product starts the shared runner yet |
| S2, S3 | Not started |

### 15.1 P0: `runner.js`, measured

`scripts/stage-runtime.mjs` writes every `src/runtime/*.ts` as `.js` with Node's own stripper (`module.stripTypeScriptTypes`, mode `strip`: types become spaces, so stack-trace lines and columns are the source's; relative `./x.ts` imports are rewritten to `./x.js`, and staging fails if one is left). The packaged host runs `<runtime>/runner.js` (`runtime_paths.rs`); the Windows installation is recognised by `resources\runtime\runner.js` (`platform/layout.rs`). A development build still runs `src/runtime/runner.ts` (`cargo run`, `--plugin` unchanged, no build step). **A stage made before this change has no `runner.js`**: a new host copied into an old stage directory is not recognised as an installation; restage the runtime (`node scripts/stage-runtime.mjs <stage>/resources/runtime`).

Method: the researcher's fake host (`memspike/measure.mjs`, `--nohook`, canned replies per plugin, `os-sample.mjs` after 5 s idle), one Node process at a time, the 17 built-ins' `dist/`, median of 3 (machine shared, about 1 GB free). `ts` = main's staged runtime (`runner.ts`), `js` = this branch's staged `runner.js`, `+ cache` = with `NODE_COMPILE_CACHE` warmed by one unrecorded run. Ready = spawn to `runtime.ready`; CPU = the process's total CPU at idle.

| 17 built-ins | ts | js | js + cache | ts + cache |
| --- | --- | --- | --- | --- |
| WS sum (MB) | 1071 | **846** (−225, −13.3 per process) | 844 | 848 |
| PB sum (MB) | 1208 | **1016** (−192, −11.3 per process) | 1022 | 1020 |
| CPU sum (ms) | 4452 | **3500** (−21%) | 3379 | 3346 |
| Ready, median of the 17 medians (ms) | 390 | **271** (−30%) | 322 | 323 |

Per plugin (ready ms / WS / PB): `ts` 306–1075 / 61.6–65.6 / 70.0–72.9; `js` 237–615 / 47.2–58.2 / 59.1–61.3. The `js` outliers (playlist 615, video 495 ready) are single noisy runs on a shared machine; the second sweep below has them at 319 and 260.

`NODE_COMPILE_CACHE` on top of `runner.js`, re-measured with N=5 on six plugins (playback, playlist, video, store, visualizer, audio): ready median 287 → 289 ms, CPU 1236 → 1251 ms, WS 300.2 → 303.3 MB. The cache's gain in §1 (−13 MB, −30 ms) was skipping the runner's type stripping, which `runner.js` already removes; the SDK's and plugins' bytecode compiles in a few ms. Below the §10.2 bar ("kept if ≥ 10 ms"), so the host does not set it. It would help a development build (`ts + cache` ≈ `js`), which is not the product; revisit for S if its single start of 17 plugins shows compile time.

§11 row 1 (18 own processes, node WS ≤ 900 MB, ready ≤ 190 ms median) is the coordinator's measurement with the installed layout. The fake-host sum (846 MB for 17) suggests the WS bar holds; the 190 ms ready bar is met by neither runner under the fake host on this machine today (271 ms; the spike measured 209–243 ms for `ts` on a quieter machine), so read the ready numbers as relative.

### 15.2 S1: the split and the shared runner

- `boot.ts`: one plugin's life (`readManifest`, `boot(options)` → `{ deliver, stop, done, stats }`); no `process.argv`, `process.env`, stdout, `process.exit` or `console`. Options: `directory, manifest, dataDirectory, locale, app, write(line), log, trace, entryLoaded`; `entryLoaded` lets the own runner install its `unhandledRejection` handler at the same moment as before.
- `runner.ts`: the process around one boot. Its stdout is byte-identical to main's: six built-ins (video, playlist, playback, audio, subtitles, window) run under main's staged `runner.ts` and this branch's `runner.js` with the same answers wrote identical bytes (video: 92 lines, 28 317 bytes), including a `runtime.failed` with exit code 1. `tests/sdk/runner.test.ts` passes; its one changed line is the packaged-layout test's runner path (`runner.ts` → `runner.js`), because the staged layout no longer contains `runner.ts` (§10.1 and §12 P0 name this change). Every other test in it is untouched and runs `src/runtime/runner.ts`.
- `shared-runner.ts`: the §5.2 protocol (`runner.hello`, `plugin.start`, per-plugin `runtime.stop` → `runner.stopped { g, timedOut }`, `runner.fault`, `runner.ping`/`pong` with `{ heapUsed, heapTotal, rss, external, plugins: { <g>: { id, requests, pending, events, handlerMs, readyMs } } }`, `runner.stop`). Every line it writes starts `{"g":<n>,` (`g` spliced into the inner JSON, no re-serialisation). Stop deadline 5 s; `VESTRA_SHARED_STOP_MS` shortens it for tests. A failed activation is disposed in S; a `runtime.stop` for a `g` S no longer holds answers `runner.stopped` at once.
- Attribution (§6.2, settled): in Node 26 `AsyncLocalStorage.getStore()` is available inside both the `uncaughtException` and the `unhandledRejection` handler for throws from timers, `nextTick`, `setImmediate`, fs callbacks and promise continuations started inside `als.run`. A plugin's entry module is evaluated inside its store, so the design's "bare `import()`-time `nextTick`" case **is** attributed. Not attributed: a listener on a process-wide emitter created outside any plugin (`process.stdin.on("data", …)`), which runs in the emitter's context → `runner.fault { g: 0 }`, exit 1. A throw from a plugin already stopped (a timer its cleanup left) is logged with its prefix and not reported again.
- Console: `log/info/debug/warn/error` → stderr with `[<plugin id>] ` (`[shared] ` without a store). `process.stdout.write` from plugin code goes to stderr with the prefix; S keeps the original for its own lines. The own runner keeps today's behaviour (§13 item 3 not applied, so its wire stays unchanged).
- Tests (`tests/sdk/shared-runner.test.ts`, 10): two plugins on their own generations and id sequences, replies for an unknown `g` or id reach nobody; a failing activation and a missing manifest leave the other plugin running; stopping one runs its cleanup while the other still runs commands, `runner.stop` exits 0 after both cleanups; a never-settling cleanup → `timedOut: true`; prefixed logs, stdout writes on stderr, an unhandled rejection logged, a 1 MiB + 1 request refused with `limit` and never written; an attributed uncaught exception stops only its plugin (one fault; a later throw only logged); an unattributed one ends S with code 1; the staged `shared-runner.js` runs without TypeScript and without an ExperimentalWarning; **plugin-API invariance**: built-ins `video` and `playlist` (their package directories, as `--plugin` loads them; `test:sdk` runs before `plugins:build`, so not `dist/`) write the same messages, minus `g`, under `runner.ts` and under the shared runner beside a neighbour plugin.

### 15.3 What S2 needs

- `runtime_paths.rs`: `shared_runner` next to `runner` (`<runtime>/shared-runner.js` packaged, `src/runtime/shared-runner.ts` in development); `--max-old-space-size=1024` on S's command only.
- The host side of §5.2 as `shared-runner.ts` speaks it: wait for `runner.hello`; `plugin.start` carries what `load_manifest` puts in the environment today (`dataDirectory`, `locale`, and `app` as a JSON value, not a string); `runner.stopped` arrives for every `runtime.stop`, including one for a member that already failed or faulted; `runner.fault { g: 0 }` is followed by exit 1.
- Envelope reader: lines up to `MAX_MESSAGE_BYTES + 64`, `g` always the first key.
- Heartbeat and stats consumers, `trust()`, `builtin.json` schema 3, restart-all with the loop guard and fail-open (§7, §8): none exist yet.
- The SDK docs sentence of §13 item 4 ships with S2.
