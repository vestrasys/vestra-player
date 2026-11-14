# Verifying an additions-only change

Two tools for a change that may only add (first used for SDK 1.8.0, plan A: nothing that exists may change behaviour). Both are plain Node scripts without dependencies; their headers have every option.

| Tool | When | Says |
| --- | --- | --- |
| V1 static gates, `src/scripts/verify-gates.mjs` | On every branch or working tree of the batch, in each of the three repositories | Whether an existing test, golden trace, API signature or capability entry was changed |
| V2 runtime differential, `src/scripts/verify-runtime.mjs` | Before merge, on two staged players (the base and the batch) | Whether any plugin's registrations, standby classification, parking, waking or recording differ |

The tools' own tests are `src/tests/sdk/verify-gates.test.ts` and `verify-runtime.test.ts` (run by `npm run test:sdk`, so by `ci:local`).

## V1 static gates

```text
node src/scripts/verify-gates.mjs --repo <repository> --base origin/main
node src/scripts/verify-gates.mjs --repo <repository> --base origin/main --head <branch>
```

Without `--head` the working tree is compared (tracked files, staged or not), so a maintainer who does not commit can run it. The base is the merge base of `--base` and the head unless `--exact-base` is given. The repository kind (player, SDK, plugins) is detected. Exit code 0: every gate passes; 1: a gate fails; 2: usage or git error.

| Gate | Repositories | Fails when |
| --- | --- | --- |
| (a) traces | SDK | A golden trace of the base (`test/fixtures/traces/`) is modified or deleted. New traces pass |
| (b) tests | all | A line of an existing test file is removed or modified, or a test file is deleted or renamed. Test files: a path with a `test`, `tests` or `__tests__` directory, `*.test.*`, `*.spec.*`, `tests.rs`, `*_test.rs`, `*_tests.rs`; the files of a module the base declares as `#[cfg(test)] mod x;` (`x.rs`, `x/mod.rs`, a `#[path]`, and everything under `x/`); and in every other Rust file the lines under `#[cfg(test)]` or `#[test]` |
| (c) api-surface | SDK | A line of `test/fixtures/api-surface.txt` is removed (a changed signature is a removed line) |
| (d) capabilities | player `src/host/capabilities.json`, SDK `src/host-capabilities.ts` (both removed in player 0.9.0 / SDK 1.9.0: n/a once the base has neither) | An existing capability or capability-free method is removed, changed (label, risk, residency, requires, methods and their `needs`) or reordered; a new entry takes an existing method, also one that an existing `prefix.*` entry serves (the host looks up the exact name first, then the first prefix in table order, so a new exact name under an existing prefix, or a new prefix placed before one it overlaps, changes who serves it); another top-level field changes. New entries are listed as notes |

An approved exception (for example the store plugin's tests in a change that alters the install dialog) is passed as `--allow <path prefix>`: its findings are still printed, under "allowed by --allow", and do not fail the gate. Name every `--allow` in the PR.

What the gates do not see:

- Whether the snapshot files are current: the SDK's own tests check `api-surface.txt` and the traces against the code; the player's `capabilities.json` was checked by `cargo test capabilities` until player 0.9.0. Run the suites as well.
- A test whose expectation lives outside a test file (a snapshot read from `doc/`, a fixture under another directory name).
- Rust code under `#[cfg(any(test, …))]` or behind a feature (`#[cfg(feature = "test-support")]`): it also builds without tests. Test modules declared with `#[cfg(test)] mod x;` inside an inline module, or in a crate root not named `lib.rs`, `main.rs` or `mod.rs`, are resolved to the wrong directory and so not seen.
- An added line that changes an existing test's meaning (an early `return`, a new `skip`). The reviewer reads the added lines of existing test files.

## V2 runtime differential

```text
node src/scripts/verify-runtime.mjs run --stage <staged player> --out <directory> [--store-plugins <plugin repository>]
node src/scripts/verify-runtime.mjs diff <directory A> <directory B> [--report <file>]
node src/scripts/verify-runtime.mjs compare --a <staged base> --b <staged batch> --out <directory>
```

A staged player is the output directory of `node packaging/bundle-windows.mjs --no-installer` (with `vestra-host.exe` and `resources\`), so each side runs its own host, runtime, SDK and built-in plugins. The store plugins come from a plugin repository checkout (`plugins/<name>/dist` when built with `npm run build`, else the source directory) and are loaded with `--plugin`; the `installed` scenario installs the built ones into the profile's library instead. Give both sides the same checkout: store plugins reach users already built.

Every run is offscreen (`VESTRA_TEST_OFFSCREEN=1`: no window) in a new profile under `<out>/raw/`, and uses only switches the host already has (`VESTRA_TEST_REGISTRY`, `VESTRA_TEST_PARK=all`, `VESTRA_TEST_WAKE`, `VESTRA_IDLE_MS`; `src/host/src/host/supervisor/probe.rs`). The four scenarios take about seven minutes per player.

| Scenario | Steps |
| --- | --- |
| `idle` | `first`: a first start, then at least 40 s idle after start-up settled (past the 30 s standby threshold). `second`: the next start of the same profile, recordings replayed |
| `park` | `park`: `VESTRA_TEST_PARK=all`, every installed plugin parked at its ready. `wake`: the next start, replayed, then all woken by `VESTRA_TEST_WAKE` |
| `file` | `warm`: a first start. `open`: a start with a file argument (a generated two-minute clip, or `--media`), played for 20 s |
| `installed` | The store plugins as packages installed from a file (development plugins are never parked). `first`: a first start, then at least 40 s idle. `wake`: the next start, replayed, then every parked plugin woken |

Compared, per scenario, step and plugin: the registry snapshots in order (commands with their declaration, shown text, bar button, state and conditions; key bindings; settings pages; drop providers; wake events; recorded marks), the host's standby lines about the plugin (`lazy: … classified lazy|resident` with its reasons, `parked (N recorded calls)`, `replayed`, `woken by`, `plugin <id>: keeps running | may be stopped while idle`, `supervisor:`, `runtime.failed`), what the plugin wrote to its console (except `HostError: plugin is stopping` and `HostError: plugin stopped`, a request on its way when the plugin was stopped), and its recording (`residentAtReady`, `calls`). For the host: `lazy: start-up settled`, each `shell: compile with N panel(s) […]` (not in `park`, where plugins park in any order between compiles; in `installed`/`first` only the first and the last, since third-party first runs reach the shell in one compile or in two), `open:`, `source: phase`. Owner numbers, times, source line numbers, GUIDs, digests, versions and the paths of the stage, profile, plugin repository and media file are normalised away; the header of the script lists them.

Since player 0.9.0 each panel's file compiles on its own and the shell no longer compiles per mount: each `panel: compile [<id>:<slot>] … (ready|failed)` line goes to a section of its own per step (`panels › compiles`, sorted, without times). `diff` lists differences there, and in the host's `shell: compile` lines, under **COMPILE PATTERN** after the result, and does not count them: against a player before 0.9.0 they are the ruled change of SDK 1.9.0 C1 (one shell compile per mount change becomes one compile per panel shown; the adjust column's front tabs at its first opening), to be read against that pattern rather than as a failure (`compilePattern`).

`run` writes `normalised.json` (what is compared), `summary.txt` (a table of every plugin's class, parks, wakes and recording per step, for people), `run.json` (versions, timings, warnings) and the raw output. A run that did not idle long enough, or whose player failed, is invalid (exit code 2) and compares nothing: repeat it with `--slow 2` on a loaded machine. `diff` exits 0 on no difference and 1 otherwise, naming each one by scenario, step, plugin and entry. It normalises both sides again from their raw output, so a baseline kept from an earlier version of the script is compared by the current rules; keep the whole `<out>` directory.

What the differential does not see:

- **Requests that leave no registration**: what a plugin asks the host and which events it is sent (the start-up traffic, event subscriptions that do not wake, reads). The host has no switch that records them; the SDK's golden traces cover the examples only.
- **Store plugins from the signed store**: the `installed` scenario writes the library's records itself (source "file"), so the store's download, signature check, install dialog and update path are not run.
- **Commands and panels in use**: nothing is clicked. Panels are mounted, but their property values, the menus as drawn, dialogs and the settings pages' contents beyond their registered values are not compared.
- **A command's state and a page's values at `ready`, `parking` and `woken`**: plugins set some after their ready, so they are compared at `parked`, `replayed` and `quit` only (as `tests/lazy_activation.rs` does).
- **Timing**: how long a start, a shell compile or a file open takes; frame rates (measured separately with `VESTRA_FRAME_LOG`).
- **Windows, GPU rendering, audio output**: the runs are offscreen, software-rendered and silent.
- **Source resolvers** in the playback worker (not in the registry probe), and anything a plugin does only for a network source.
