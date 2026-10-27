# Session storage (`plugin.storage.session`): SDK 1.5.25, host 0.5.5

Status: implemented by the API maintainer; independent review "merge, with low items to fix first" (memory bound, property order, per-value limit, dropped instances, wording, a rejected first set, lone surrogates), all fixed; awaiting release (SDK branch `sdk-1.5.25` on SDK `main` 1.5.24, d93b1dc).

## Ruling

User ruling, 2026-10-27: following Chrome MV3's `chrome.storage.session`, add an API for state that lives as long as the **player process**, not the plugin process. It survives a plugin restart (a reload from the store, a crash and restart of the shared runner planned in [2026-10-shared-runner](2026-10-shared-runner.md), the idle stop under [2026-10-lazy-activation](2026-10-lazy-activation.md)) and is cleared when the player quits. Existing APIs (`plugin.storage.json` and the rest) do not change. The docs say, where they explain the plugin lifecycle: "keep state that must survive a restart in session storage (or persistent storage)".

Under §1.6 this is an **addition**: a new member, new wire methods, no change to any existing signature, error code, default or lifecycle. Released as an SDK patch; `apiVersion` stays `vestra/1`.

## Shape

```ts
plugin.storage.session.get<T = unknown>(key: string): Promise<T | undefined>
plugin.storage.session.set(key: string, value: unknown): Promise<void>
plugin.storage.session.delete(key: string): Promise<void>
plugin.storage.session.keys(): Promise<string[]>
```

`Storage` gains `readonly session: SessionStorage`; `SESSION_STORAGE_LIMITS` (`keyBytes` 1024, `bytes` 1 MiB) is exported from `/testing` and `/runtime`. Wire: `storage.session.get { key }` → `{ value }` or `{}`, `storage.session.set { key, value }`, `storage.session.delete { key }`, `storage.session.keys` → sorted keys.

Why `get/set/delete/keys` and not a `session.json(name, { defaults, validate })` mirroring `storage.json`:

- `storage.json` is synchronous (read on creation, `value` is a property) and saves **debounced** (300 ms). Over the wire the read is necessarily async, and a debounce would lose exactly the write that session storage exists for: a crash within the debounce window. Each `set` here is one acknowledged request; when its promise resolves the value is in the player.
- The existing host-backed per-plugin store, `plugin.secrets`, is already `get/set/delete` of named values with async calls; session storage follows it (plus `keys()`, which secrets lacks because the system store cannot list cheaply).
- It matches `chrome.storage.session` (`get/set/remove/getKeys`), which the ruling names.
- `undefined` for a missing key (not `null` as `secrets.get`): `null` is a valid JSON value here.

## Semantics

| Event | Values |
| --- | --- |
| Plugin reloaded (`packages.reload`) | kept |
| Plugin upgraded (same installation instance) | kept |
| Plugin process crashed / stopped by a fault, started again | kept |
| Start failed, or the plugin is in the failed state, and is started again | kept (nothing clears on a failure) |
| Development load (`--plugin`) reloaded | kept (development key) |
| Shared runner restarts all its members (§8.2 of the runner record; a later player) | kept (the host calls `disable(owner)` and starts again; nothing clears) |
| Idle stop and wake (lazy activation, a later player) | kept |
| User disables the plugin (`packages.setEnabled(id, false)`) | **cleared** |
| User uninstalls the plugin (with or without `removeData`) | **cleared** |
| Player quits | cleared (memory only) |
| A retired instance is dropped from the full dormant list (`purge_dropped_secrets`) | **cleared**, with its secrets: it can never be reconnected |
| Plugin replaced by the same id from another source (new instance) | the new instance starts empty; the old instance's values stay in memory until quit and are seen again if the user switches back within the session (as its data directory is) |

Decisions:

- **Key:** the plugin's `DataKey` (installed instance GUID, or the development id), never the process generation (`reply.owner`). This is the identity of the data directory and the secrets service, so "per plugin" means the same thing for all three stores, and a development load (`--plugin`) of an installed id has its own values.
- **Disable by the user = clear; any other stop = keep.** Disabling is the user switching the plugin off, the analogue of Chrome clearing session storage on disable. The only user disable path is `packages.setEnabled(false)` (`package_manager.rs::set_enabled`), so the clear sits there; `Host::disable(owner)`, which every stop goes through (reload, crash, protocol fault, shared-runner restart), does not clear.
- **No capability**, like `plugin.storage.json` (the host's `required_capability` returns `None` for the four methods).
- **Isolation:** the host derives the key from the requesting plugin; a plugin cannot name another's. Requests from a stopped owner are dropped before dispatch (the owner leaves `Host::plugins` synchronously in `disable`), so a late `set` cannot repopulate a cleared store.
- **Values:** any JSON. The SDK sends `JSON.parse(JSON.stringify(value))`, so what `get` returns is what `JSON.stringify` wrote; a value it cannot write (`undefined`, a function, a cycle, a `BigInt`) is `invalid` before sending. The host does not validate shape; an upgraded plugin may read an older version's value (documented: check the shape).
- **Stored as compact JSON text** (`String`), not as a `serde_json::Value` tree: a `Value` costs 32 bytes per node, so a 1 MiB array of zeros would have taken about 16 MiB. `get` parses the text again (it was written from a `Value`, so it always parses).
- **Object property order is not kept.** The host parses the request into `serde_json::Value` without `preserve_order`, so objects come back with their properties sorted (JavaScript then lists integer-like names first, as always). Documented on `SessionStorage`; `TestHost` stores objects sorted the same way so tests see what the player returns.
- **Lone surrogates.** `JSON.stringify` writes a lone surrogate as `\udXXX`, which `serde_json` refuses; the host then cannot parse the line and stops the plugin with a protocol fault. The SDK refuses such a key or value with `invalid` before sending (a `u`-flag regex, since `String.prototype.isWellFormed` is beyond the SDK's ES2023 target). This is a general wire issue, not one of session storage: any request carrying a plugin string with a lone surrogate (an OSD text, a property value, a command title) has the same effect. Left for a separate round: either the runner rejects such requests with `invalid`, or the host parses leniently and replaces them with U+FFFD.
- **Keys:** 1–1024 UTF-8 bytes, no NUL, otherwise free text (session state is often keyed by a URL or a path, which the `a-z 0-9 -` rule of `storage.json` names would not allow).
- **Quota: 1 MiB per plugin**, counting every key's bytes, its value's compact JSON and `ENTRY_BYTES` (128) per key for the map's own bookkeeping (two `String` headers in a `BTreeMap` node at ≥ half fill plus two allocations, about 100–150 bytes). With values held as text and the per-key charge, the quota is the memory: a plugin costs the host about 1 MiB at most (a little more for keys whose real bookkeeping exceeds 128 bytes; at most 8192 keys fit), so 17 built-ins plus a few third-party plugins stay around 20 MiB in the worst case, and real use is far below. Chrome allows 10 MB for all of an extension's session storage (raised from 1 MB in Chrome 112); a player plugin is smaller in scope. **One value is also bounded by the wire**: a `set` request is one message of at most 1 MiB (`MAX_MESSAGE_BYTES`, enforced by the runner before sending, `runtime/runner.ts`), so a single value is 1 MiB minus its key and the request envelope; a larger one fails `limit` in the runner, and `TestHost` now refuses every request over the message limit the same way. The docs say to split large state over several keys. Overflow fails with **`limit`** (the existing code for a count or size cap, as `secrets.set` beyond 256 names and the image budget) and changes nothing; replacing a key counts the new value instead of the old one.
- **Older players** reject the methods with `unsupported` (unknown method); the docs say to set `"minPlayerVersion": "0.5.5"`.

## Alternatives considered

1. **`session.json(name, { defaults, validate })` mirroring `storage.json`.** Familiar, but async open plus debounced save loses the last write on a crash, the very case the ruling targets; a non-debounced variant would be a `JsonStore` with different save semantics under the same type name. Rejected.
2. **Keep it in the plugin's Node process / a file in the data directory cleared at player start.** A file survives the player quitting unless the player deletes it at start, which needs a host hook anyway and leaves state on disk after a crash of the player; memory in the host is simpler and cannot leak across runs. Rejected.
3. **Key by process generation (`owner`).** Would not survive a restart. Rejected by construction.
4. **Clear on reload too (Chrome clears on extension reload/update).** The user ruling names reload as a restart to survive. Rejected.
5. **Larger quota (10 MB like Chrome).** Not needed by any current plugin; a later patch can raise it without breaking anyone (`SESSION_STORAGE_LIMITS` documents the figure).

## Changes

Player (`session-storage` branch, host 0.5.5):

- `src/host/src/session_storage.rs` (new): the store, the four requests, `clear`, unit tests (round trip, missing keys, bad keys, missing value, isolation between plugins and between an installed instance and a development load, quota counting with the per-key charge on realistically sized values, many tiny entries, refused set changes nothing, a refused first set leaves no entry, delete returns bytes, values held as compact text with sorted objects).
- `src/host/src/main.rs`: `mod session_storage`, the `Host::session_storage` field, routing `storage.session.*` in `dispatch` (keyed by `plugin.data_key`), `None` capability arm in `required_capability`.
- `src/host/src/package_manager.rs` (two small insertions, for an easy merge with the shared-runner branch): in `set_enabled`, the `!enabled` branch clears the stopped plugin's key (development key when a development load is the one switched off, else the instance key); in `uninstall`, after `failures.remove`, clears the instance key; `purge_dropped_secrets` clears each dropped retired instance with its secrets.
- `src/host/Cargo.toml`, `Cargo.lock`: 0.5.5 (0.5.4 is the deinterlace branch).
- `src/tests/sdk/host-agreement.test.ts`: one appended test, `SESSION_STORAGE_LIMITS` equals `MAX_KEY_BYTES` / `MAX_SESSION_BYTES`; it skips while the installed SDK is older than 1.5.25.
- `doc/architecture/README.md`: a row for `session_storage.rs`.

SDK 1.5.25 (patch on SDK `main`): `SessionStorage` and `Storage.session` (types), `SESSION_STORAGE_LIMITS` and the implementation (`services.ts`), the wire client (`wire.ts`), `TestHost.sessionStorage` with the player's key rule and quota and `startPlugin(..., { sessionStorage })` to test a restart, `test/session-storage.test.ts`, docs (*How a plugin runs* gains *Restarts: activate can run more than once* with the emphasis and a table of what survives what; *Test a plugin*; *Reference: limits*; generated tables, `llms.txt`), CHANGELOG ("Who must change: nobody").

## Verification

- Host unit tests (`session_storage::tests`, 7) and the whole `cargo test --locked`.
- Headless end-to-end run of the debug host with two development plugins (a counter that increments `runs` in session storage on each activation, and a manager with `plugins.manage`): `runs=1`; the manager's own `keys()` shows only its key; after `packages.reload` `runs=2`; after `setEnabled(false)` then `setEnabled(true)` `runs=1`; a 1 MiB value fails `limit` each time.
- Not exercised end to end: uninstall (needs an installed package); covered by the unit test of `clear` and the four-line call site.

## Release order

SDK 1.5.25 after 1.5.24 (already on SDK `main`); the player then moves to `^1.5.25` with host 0.5.5. The workshop's vendored SDK (`plugins/workshop/vendor/`) must be refreshed with `npm run vendor:sdk` when the player upgrades, as for every SDK release (its test "vendor/ matches a fresh build" fails until then).
