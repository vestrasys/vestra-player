# 2026-10: Where a plugin process's memory goes (researcher spike)

Role: researcher (read-only). Measured 2026-10-27 on the Windows laptop (Node v26.12.1, free RAM 0.8–1.8 GB, machine shared; every measurement sequential, peak < 0.7 GB). Scripts and raw data stayed in the coordinator's scratchpad (`memspike/`: `measure.mjs` fake host that starts `runner.ts` the way `host/src/plugins.rs:136-152` does, `memhook.mjs` `--import` hook recording `process.memoryUsage()` and the loaded modules, `shared.mjs` for the K-plugins-in-one-process models, `out/report.md`). Numbers are medians; WS = working set, PB = private bytes (committed), in MB.

The question behind it: 18 built-ins ≈ 1.1 GB of Node processes while PotPlayer with everything is ~200 MB. The user's ruling after reading this: lazy activation (see `2026-10-lazy-activation.md`) handles the *number* of installed plugins; a **shared process for trusted plugins** (`2026-10-shared-runner.md`) handles the *resident* cost; trust is decided by the host from provenance, never from a manifest field; the plugin API does not change.

## 1. Baseline → SDK → runner → real plugins (N=3, 10 s idle)

| Process | Ready ms | WS | PB | Threads | heapTotal | external | Modules |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `node` idle | 139 | 41.5 | 20.7 | 13 | 6.6 | 2.2 | 1 |
| idle + libuv thread pool woken (`fs.promises.readFile`, no SDK) | 140 | 42.3 | **53.6** | 17 | 6.9 | 2.3 | 3 |
| SDK runtime imported (`player-runtime.js`, 44 modules / 308 KB) | 173 | 47.0 | 25.2 | 13 | 10.6 | 2.7 | 44 |
| real `runner.ts` + tiny plugin | 217 | **62.6** | 70.1 | 17 | 15.7 | **6.3** | 47 |
| type-stripped `runner.mjs` + tiny plugin | 177 | 47.9 | 58.4 | 17 | 11.4 | 2.6 | 47 |
| 18 built-ins (`runner.ts`), range | 209–243 | 62.8–65.2 | 70.4–72.5 | 17 | 16.0–17.7 | 6.3–6.5 | 47–63 |
| 18 built-ins, sum / mean | — | **1146 / 63.7** | **1277 / 71.0** | | | | |

Where a typical 63 MB goes:

- 41.5 MB node.exe itself, of which only 20.7 MB is private; the other ~20 MB are node.exe image pages shared by every process but counted in each working set. "17 × 60 MB ≈ 1 GB" double-counts about 17 × 20 MB; exclusive physical memory is closer to 0.8 GB.
- **+14.7 MB WS / +11.7 PB / +3.7 external: `runner.ts` run as TypeScript.** Node loads Amaro (SWC in WASM) in every process to strip types; `external_script_source_size` 2.5 → 6.2 MB, heapTotal +4 MB. The largest single item and the cheapest to remove.
- +5.5 MB WS: the SDK runtime (38 modules, 308 KB; every plugin loads the same set because `createPlugin` imports all capability modules).
- +0.2–2.6 MB: the plugin's own code. The 18 built-ins have no `node_modules`; sources are 7–249 KB (nes `main.js` 249 KB); workshop's 27.7 MB `vendor/` is not loaded at activation.
- +33 MB PB (not WS): the libuv thread pool, 4 threads × ~8 MB stack commit, woken by the runner's `readFile`. `UV_THREADPOOL_SIZE=1`: PB 72.5 → 49.0, WS unchanged.
- Built-in Node modules used by all plugins: `readline fs/promises path url net crypto child_process` (the last three via the SDK's `services/local/stream`); capture/info add `os`, subtitle-online/workshop add `zlib`. No heavy dependencies anywhere; the plugins hardly differ.

## 2. Node flags (playback plugin, N=3)

| Flag | Ready ms | WS | PB | Note |
| --- | --- | --- | --- | --- |
| none | 230 | 65.3 | 73.0 | |
| `--max-semi-space-size=1` | 237 | 62.6 | 69.5 | −2.7 / −3.5 |
| `--optimize-for-size` | 250 | 63.5 | 69.9 | −1.8 / −3.1 |
| `--single-threaded` | 252 | 65.4 | 72.1 | no effect (the pool is libuv's) |
| `--jitless`, `--lite-mode` | fails | | | `ERR_WEBASSEMBLY_NOT_SUPPORTED`: type stripping needs WASM |
| `--jitless` + JS runner (N=1) | 254 | 45.0 | 58.5 | −6 MB over the JS runner, +60 ms |
| JS runner, no flags | 192 | **50.9** | **61.6** | −14.4 WS / −11.4 PB / −38 ms |
| `NODE_COMPILE_CACHE=<dir>` with `runner.ts` | 200 (miss 237) | **51.9** | **61.7** | the cached strip result skips Amaro: same gain without code changes |

## 3. Startup snapshot (esbuild-bundled SDK runtime, N=2)

| | Ready ms | WS | PB |
| --- | --- | --- | --- |
| bundled SDK, normal load | 156 | 41.8 | 23.3 |
| `--snapshot-blob sdk.blob` (9.3 MB) | 147 | 51.4 | 30.1 |

Buildable (456 ms) but `--build-snapshot` accepts only built-in `require`s, so user code must be one CJS file (no ESM), and `node:child_process` warns. −9 ms for +7–10 MB resident: **not a lever**; the 300 KB SDK costs ~30 ms anyway.

## 4. K plugins in one process (totals include the fake host 42.3 WS / 21.5 PB; N=2)

| Model | K=1 | K=4 | K=8 | K=18 | Marginal per plugin | 18 plugins start, wall / CPU |
| --- | --- | --- | --- | --- | --- | --- |
| one isolate (WS / PB) | 51.2 / 60.8 | 57.5 / 66.4 | 58.9 / 68.4 | **65.6 / 73.3** | **0.8 / 0.7** | 197 ms / 226 ms |
| K `worker_threads` | 60.8 / 44.1 | 111.2 / 111.5 | 172.2 / 194.2 | **324.7 / 434** | **15.5 / 22.9** | 1003 ms / 1055 ms |
| K real `runner.ts` processes | 108.6 / 94.8 | 302.7 / 308.6 | 559.7 / 591.7 | deferred (RAM); ≈ 1190 by the sum | **64.5 / 71.0** | 8 plugins: 1254 ms |

All 18 built-ins activated in one isolate twice without an error. SDK module state is per instance (WeakMaps keyed by the plugin object, id counters: `sdk/dist/plugin/commands.js:47`, `language.js:45`, `player.js:8`, `subtitles.js:6-7`, `markdown.js:27`, `ui.js:12`, `services.js:105-110`); `process.*` is only read for `platform/pid/env.PATH/VESTRA_FFMPEG`. What blocks a shared runner today is the runner and the host protocol:

- `src/runtime/runner.ts:26` `process.argv[2]` (one plugin dir), `:28` one manifest; `:18-19,57,93` `process.stdout.write` and `:125` `createInterface({input: process.stdin})` (the transport is the process's pipes; wire messages carry no plugin id, the host keys on the child: `host/src/plugins.rs:194`); `:23-25` global `console` override; `:113-122` `stop()` → `process.exit(0)`, `:161` `process.exitCode = 1`, `:217` activation failure → `process.exit(1)`; `:183` process-wide `unhandledRejection`; `:170,188,191` per-plugin parameters from `VESTRA_APP_INFO/VESTRA_LOCALE/VESTRA_PLUGIN_DATA_DIR` (set per child in `plugins.rs:139-145`).
- Host: `plugins.rs:1` "one Node process per trusted plugin", `:136-152` spawn per `load_manifest`, `:264-272` stop/timeout = kill the child, `MAX_LINE` per process.
- Plugins: temp names built from `process.pid` (`capture/dist/copy.js:14`, `hotkeys/dist/keys-page.js:451`, `info/dist/dialog.js:172`, `playlist/dist/listfile.js:108`, `visualizer/dist/covers.js:324,354`, `logo.js:80`): unique per process today, not per plugin in a shared process; the paths do not overlap, so no conflict yet. Workshop's own child-process handling (`worker/worker.ts:214,231,242`) is unaffected.

Chrome for comparison (cited, not measured): Chromium's process-model docs give no per-renderer figure; commonly observed empty renderers are 20–40 MB private, the same order as node idle (20.7 PB / 41.5 WS). A Node process is not dearer than a Chrome renderer; what we add on top is.

## 5. Conclusion

Lazy activation and a shared process are the order-of-magnitude levers (process 64.5 → worker 15.5 → one isolate 0.8 MB per plugin); SDK slimming and snapshots have no room (SDK 5.5 MB; snapshot +7 MB). Without changing the isolation model, the cheapest step is a precompiled JS runner or `NODE_COMPILE_CACHE` (−14 MB WS / −11 MB PB / −40 ms per process, ≈ −250 MB for 18) plus `UV_THREADPOOL_SIZE=1` for commit. For built-ins, one shared isolate turns 1.1 GB into about 70 MB; the SDK runtime is not an obstacle, the obstacles are `runner.ts` and the host's one-child-one-plugin protocol.

Not run: 18 real processes at once (RAM < 3 GB free both times), a third round of the snapshot and shared models (the sweep was OOM-killed once), `--max-old-space-size`, any GUI or cargo build. `src/plugins/` holds 18 manifests (the lazy-activation doc says 17); this report counts 18.
