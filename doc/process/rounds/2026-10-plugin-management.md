# 2026-10 round: plugin management (built-ins in place, no rollback)

Started 2026-10-26. Top-down API change (community-workflow §5). **This document is the only progress record for this round.** Status: P1 design written by the API maintainer. The user answered Q1–Q3 on 2026-10-26 (§6). P1 implementation is next.

Line references are to `main` at `2d50207` unless marked `6bf7e77` (the stop-gap commit on branch `startup-seeding`, which merges first). SDK references are to the installed `@vestra-player/sdk` 1.5.15 (`src/node_modules/@vestra-player/sdk/dist/...`) and, by file, to the SDK repository.

## 1. Ruling and goals

User ruling, 2026-10-26 (binding):

- **Plugin rollback is removed entirely**, for built-ins and third-party plugins alike. To get an older third-party version, the user uninstalls it and installs a `.vxp`. (VS Code has no rollback either.)
- **Built-in plugins** cannot be uninstalled, only disabled. Users do not manage their versions. They **run in place from the installation directory**, with no copy into `%APPDATA%\Vestra\packages`, and update with the player.
- **Third-party plugins** can be uninstalled.

Why it matters: built-ins are copied ("seeded") into the user library so that a user's rollback of a built-in survives an update (`package_manager.rs:388-496`, the `seededVersion >= version` test at :425-431). On Windows that froze the UI for 14–21 s at first start and after an update. The stop-gap `6bf7e77` moves the copy to a `builtin-seed` thread (`package_manager.rs@6bf7e77:406-535`). This round removes the copy itself.

**Acceptance (whole round)**

| # | Check |
| --- | --- |
| A1 | First start and first start after an update: no UI freeze, and no file is written under `packages/packages/` for any built-in |
| A2 | A built-in can be disabled and enabled, the state survives a restart, and `uninstall` answers `unsupported` |
| A3 | A third-party plugin can be uninstalled, with or without its data |
| A4 | No rollback anywhere: no `packages.rollback` on the wire, no `Packages.rollback` / `previousVersion` in the SDK, no Roll Back button or text in any plugin, and no previous version kept on disk |
| A5 | Existing users keep their built-in settings, secrets and disabled state, and their third-party plugins and data |

## 2. Host design

### 2.1 Built-ins ship unpacked and run in place

**Decision: unpacked directories, not archives read in place.** Node cannot run a plugin from a zip. Reading an archive "in place" would still mean extracting it on every start. Today that extraction plus `Candidate::capture` copies every file into a temp directory (`package_stage.rs:41-105`), and that copy is the slow part on Windows.

Layout (`<resources>` as in `platform/layout.rs:10,67,83,119,170`, unchanged):

```text
<resources>/plugins/
  builtin.json          { "schema": 2, "plugins": [{ "id", "version", "directory" }] }
  vestra.audio/         the unpacked content of the .vxp (manifest.json, main.ts, ...)
  vestra.workshop/
  ...
```

- `scripts/stage-plugins.mjs:48-79` keeps running `plugin.mjs pack` for each `src/plugins/<name>`. It then unpacks each `.vxp` into `<id>/` and deletes the `.vxp`, so the shipped tree is byte-for-byte what an install would get. It writes schema 2. `directory` is a single path component that must equal `id`.
- The host reads `builtin.json` (the `seed` module at `package_manager.rs:1118-1181` becomes a `builtin` module) and accepts only schema 2. Each entry is checked:
  - the id is a **reserved** id (`manifest::reserved_id`, `manifest.rs:152`). This is new, and it keeps "official = `vestra.`" true by construction. All 18 built-ins already comply.
  - `<directory>` is a real directory (not a symlink) whose canonical path lies under the canonical `<resources>/plugins`;
  - `manifest.json` parses and passes `manifest::check` like a development load (`manifest.rs:131-138`), and its `id` and `version` equal the entry.
- These checks read one small file per plugin. No file is hashed or copied. An entry that fails is logged and listed as `failed`, as a failed start is today.
- **Integrity.** Nothing replaces the archive `sha256` at run time. The plugin tree gets the same trust as everything else in `<resources>` that the host already runs unverified: `runtime/runner.ts`, `ui/host.slint` and the bundled Node (`runtime_paths.rs:56-66`). The installation protects it:
  - macOS: the `.app` code signature seals `Contents/Resources`;
  - Windows: the Inno Setup install directory is admin-writable only;
  - the portable zip is as trusted as the `.exe` next to it.

  At build time, `stage-plugins.mjs` checks the unpacked tree against the `.vxp`, and `packaging/smoke.mjs:22-24,101` (CI `_package.yml:91,104`) still requires every built-in to reach `ready`.
- **Read-only directory.** The host never writes under `<resources>/plugins`. Plugins already write only to `plugin.dataDirectory`. The workshop, the only built-in that touches its package directory, reads `vendor/` and copies it into its data directory (`plugins/workshop/toolchain.ts:192-231`). A host test runs a built-in from a read-only copy (P1 acceptance).

### 2.2 Ids, data directories, disabled state

The plugin's data directory and secrets hang on the instance GUID (`plugins.rs:23-44`: `plugin-data/<GUID>/`, `vestra.plugin.<GUID>`). The GUID must therefore survive.

**Built-in record.** The catalog keeps one entry per built-in in `installed`, with no package:

```json
"vestra.audio": { "revision": 7, "current": null, "previous": null,
  "instance": { "guid": "…", "source": { "kind": "bundled" }, "builtin": true, "installedAt": … } }
```

- This adds no new catalog field, on purpose. The `Catalog`, `Installed` and `Instance` structs use `deny_unknown_fields` (`package_store.rs:57-70,116-125,139-162`), so this shape is still read by an older player, which then just reseeds (see §2.3, downgrade).
- A fresh profile gets its built-in records (new GUIDs) in **one** catalog publish at start: a small atomic write, no package files.
- **Disabled state** stays in the id-keyed `disabled` set (`package_store.rs:152-154,654-670`). `set_disabled` already requires only that the id is in `installed`.
- **Start / list / enable**:
  - the start loop (`package_manager.rs:863-886`, `start_installed` at `@6bf7e77:540`), `set_enabled` (`:682-693`) and `package_list` (`:595-622`) take a built-in's directory and manifest from the `builtin.json` registry, not from `current`;
  - `is_builtin(owner)` (`:498-507`) compares with the registry directory;
  - `PluginInfo` for a built-in is unchanged in shape: `origin: "installed"`, `builtin: true`, `source: bundled`, and `directory` = `<resources>/plugins/<id>`.
- **Dev-load override** is unchanged. A `--plugin` load of the same id wins for that run and uses `DataKey::Development` (`main.rs:551-604`; start loop `:869`; `set_enabled` `:663-670`). `$VESTRA_BUILTIN_PLUGINS` (`runtime_paths.rs:82,92`) still points at a staged directory, now schema 2.
- A built-in record whose id is no longer in `builtin.json` (a built-in that was dropped) is retired to `dormant` (`retire`, `package_store.rs:752`). Its data is kept, and it is neither started nor listed.

### 2.3 Migration of existing libraries

This runs once per profile and is idempotent.

1. **Catalog rewrite** (in `Store::open`, next to the existing instance migration at `package_store.rs:216-232`; one atomic `publish`, catalog ≤ 2 MiB):
   - every `installed` entry whose `instance.builtin` is true gets `current: null`, `previous: null` and no `seededVersion`. GUID, `installedAt` and the `disabled` flag stay;
   - every other entry gets `previous: null`;
   - a `pending` recovery whose target is a built-in (an interrupted seed) is dropped.

   This is one small file write, of the same kind as `setEnabled`. It never reads package files.
2. **File cleanup off the UI thread.**
   - The packages that are no longer referenced are old built-in copies and old previous versions. The `packages/<digest>` directories they name are removed by a `library-cleanup` thread, not by `Store::gc` on the UI thread (`package_store.rs:600-638`, which `install` calls at `package_manager.rs:242`).
   - The thread gets the list of unreferenced digests computed at spawn. For each one it renames the directory to `packages/.trash-<digest>` and then deletes it.
   - While it runs, install and uninstall answer `busy`, reusing the stop-gap's `library_busy` (`package_manager.rs@6bf7e77:591`).
   - A failed delete, for example while antivirus holds a file, is left for the next start.
   - From then on, a committed upgrade hands the replaced directory to the same thread.
3. **Downgrade.** An older player that opens the migrated catalog:
   - starts normally, because no field is unknown to it;
   - finds the built-ins with `seededVersion` absent and reseeds them, as it does today (`package_manager.rs:424-431`), into the same GUID;
   - keeps the third-party packages, which are still referenced by `current`, so its `gc` does not delete them.

   Re-upgrading runs step 1 again.

### 2.4 Rollback removal in the host

- `packages.rollback` is removed (`package_manager.rs:200-269`: the match arm, the `else` branch at :244-250, and the comment at :256). It then falls to `unsupported` "unknown package method" (`:307`).
- `Store::commit_with_receipt` stops filling `previous` (`package_store.rs:530-540`). `gc` and `validate_catalog` stop following it (`:280-290,602-608`).
- `Installed.previous` stays as a field that is always `null`, so that older players still read the catalog (§2.3).
- `packages.list` drops `previousVersion` (`package_manager.rs:574-580,616,633`) in P4. Until then it sends `null` (compatibility layer, §4).
- **Install-failure recovery is unaffected.** `restoring` reloads `op.old`, which is still the committed `current` until the commit (`package_manager.rs:1012-1037`, `package_store.rs:530-546`). `PackageChangeStatus` `restoring` and `failedDegraded` keep their meaning.
- Removed with the copy:
  - seeding (`seed_builtins`/`seed_one` `:388-496`; the stop-gap's `start_seeding`/`seed_tick`/`commit_seed`/`seed::decide`/`seed::stage_all` `@6bf7e77:406-535,1210-1300`);
  - `Store::set_seeded` (`package_store.rs:734-747`) and `commit_seeded`, `library`, `referenced` (`@6bf7e77` `package_store.rs:364-380,641`);
  - `Instance.seeded_version` (`:67-69`). It is still read, for migration, but never written.
- Tests that assert rollback are rewritten to assert that it is absent:
  - `package_manager.rs:1295`, `:1315-1326`;
  - `package_store.rs:882-921` (`install_upgrade_rollback_…`), `:1182-1218` (`seeded_version_…`);
  - the `builtin.json` strictness test at `:1467-1488` moves to schema 2.

### 2.5 `vestra.` reservation and "no other source for a built-in"

These are unchanged and still enforced at the same points:

- `validate` and `validateArchive` refuse a reserved id (`package_manager.rs:139`), and so does `install` (`:241`), through `check_reserved` (`:1104-1115`). An installed non-built-in record with a reserved id is not started (`:872`).
- `source: bundled` is refused from plugins (`install_source`, `:1064-1068`).
- An install over a built-in record answers `unsupported` "built-in plugins are updated with the player" (`install_instance`, `:1191-1197`). This still holds because the built-in record keeps `instance.builtin`.
- `uninstall` of a built-in answers `unsupported` (`:719-724`).
- New: `builtin.json` itself may list only reserved ids (§2.1).
- `check_reserved`'s doc comment changes from "which seeding alone creates" to "which `builtin.json` alone creates".

## 3. API change

| Surface | Change | Where |
| --- | --- | --- |
| `Packages.rollback(id)` | **Removed** | SDK `src/plugin/types.ts` (dist `types.d.ts:4830-4831`), `src/plugin/packages.ts` (dist `packages.js:129`) |
| `PluginInfo.previousVersion` | **Removed**; deprecated and always `null` during migration | `types.d.ts:4730-4731`, `packages.js:155`, `testing.d.ts:1331` |
| Wire `packages.rollback` | **Removed** (host answers `unsupported`) | SDK `src/wire.ts` (dist `wire.js:281`, `wire.d.ts:570`); host `package_manager.rs:200` |
| Wire `packages.list` field `previousVersion` | `null` in P1, absent from P4 | host `package_manager.rs:580,616,633` |
| Capability `plugins.manage` | Same capability, one API fewer: drop `Packages.rollback` from its API list | SDK `src/capabilities.ts` (dist `capabilities.js:133`); `src/tests/sdk/host-agreement.test.ts:580`; `llms.txt:63` |
| Error `revisionConflict` | Kept (install and uninstall still send `expectedRevision`); wording loses "rollback" | `types.d.ts:93` |
| Error `missing` "no previous successful version" | Gone with the method | host `package_manager.rs:249` |
| `PackageChange` / `PackageChangeStatus` docs | "install, rollback or uninstall" → "install or uninstall"; statuses unchanged | `types.d.ts:4696-4700` |
| `Plugin.dataDirectory` docs | "kept across … upgrades and rollbacks" → "… and upgrades" | `types.d.ts:339` |
| `PluginInfo.directory` docs | Add: a built-in's directory is inside the installation and read-only | `types.d.ts:4728-4729` |
| TestHost | Drop the fake `packages.rollback` and the `previousVersion` bookkeeping | SDK `src/plugin/testing.ts` (dist `testing.js:3208`) |
| SDK tests | `test/store.test.ts`, `test/round3a.test.ts` (use `previousVersion`) | SDK repository |

**Breaking?** Yes, for code that calls `rollback` or reads `previousVersion`: TypeScript fails to compile, and the call fails at run time with `unsupported`. Only holders of `plugins.manage` are affected:

- in this repo: store, plugin manager and workshop, plus a test fixture in info;
- in `vestra-player-plugins`: none (GitHub code search for `rollback` and `previousVersion`, 2026-10-26: no hits).

**Version proposal: SDK 1.5.16 (deprecate) → 1.5.17 (remove), `apiVersion` stays `vestra/1`.** Reasons:

- The user's rule is patch bumps by default.
- There is a precedent: 1.5.8 removed `PackageCandidate.trial()` at a patch level with `vestra/1` kept (SDK `CHANGELOG.md` 1.5.8).
- `apiVersion` is a load gate (`package_manager.rs:146-151`). Changing it would refuse every third-party package, youtube and bilibili included, though none of them uses rollback.

**This conflicts with community-workflow §1.6 step 2** ("a breaking change bumps the major version and changes `apiVersion`"). See question Q1.

**Docs that change** (https://vestrasys.github.io/vestra-player-sdk/, SDK repo `docs/`):

- `docs/reference/capabilities.md`: the `plugins.manage` row;
- `docs/how-to/publish.md`: "CI, the user's rollback and revocation are the safety net" becomes "CI and revocation";
- the generated API reference: `Packages`, `PluginInfo`, `PackageChange`, the error table and `Plugin.dataDirectory`;
- `llms.txt` / `llms-full.txt`, regenerated;
- `CHANGELOG.md`.

In this repo (maintainer):

- `doc/architecture/store-api-proposal.md`: rows 4 and 8, §1 table row `rollback` (:87), §2.6, §3.3, :218, :439, :491-500, :513-514, :526, :533;
- `doc/architecture/README.md` and `sdk-design.md` where they mention rollback.

`doc/product/ecosystem-plan.md:103` belongs to the interaction lead or coordinator.

## 4. Plugin migration (§5)

This is not an equivalent migration in one respect only: the ruling removes rollback. Migrators remove rollback and change nothing else (§5.2).

**Compatibility layer, P1 → P4.** The host P1 build:

- answers `packages.rollback` with `unsupported`;
- lists `previousVersion: null`;
- keeps the SDK 1.5.16 symbols, marked `@deprecated`.

Every unmigrated plugin hides its own rollback UI, because each one gates the UI on `previousVersion`:

- store `store.ts:562`;
- manager `manager.ts:161,180`;
- workshop `delivery.ts:235`. Here the null makes Undo fall back to uninstall, which is the one behavior regression the workshop must fix.

The layer ends in P4.

| Plugin (author) | Must change | Order |
| --- | --- | --- |
| **Plugin Manager** `vestra.plugins` (**maintainer**, the §5 P1 reference plugin) | See the plugin manager list below | 1 (P1) |
| **Store** `vestra.store` (store author) | See the store list below | 2 (P3, parallel with the workshop) |
| **Vestra Player AI** `vestra.workshop` (workshop author) | See the workshop list below | 2 (P3) |
| **Info** `vestra.info` (info author, or the plugin-manager migrator; 1 line) | `tests/about.test.ts:23` drops `previousVersion` from the fixture | 3 (P3, after 1.5.17-rc) |
| `zysilm.youtube`, `zysilm.bilibili` (plugin repo) | Nothing; run their CI with the new SDK (§6.1) | P4 |

**Plugin Manager** (`vestra.plugins`):

- `manager.ts`: remove the `rollback` handler (:65), `canRollback` (:161), the "previous" detail line (:180) and `#rollback` (:261-271), and fix the comment at :299;
- `ui/plugins.slint`: remove `can-rollback` (:215), the `rollback()` callback (:223), the Roll Back button (:263, :369), the "previous" label code (:124, :259) and the rollback texts (:88-95, :154-157);
- `README.md:3,16,22` and `manifest.json:15`: drop "roll back";
- locales: remove the strings;
- tests `plugins.test.ts:27,38,243,272-283,355,362,378,393-404`: remove the rollback cases, and for built-ins assert "disable yes, Uninstall… off with hint, no Roll Back".

**Store** (`vestra.store`):

- `store.ts:561-569` (the rollback/restore action) and `:708-709`;
- `installer.ts`: remove `rollback`/`rollbackVerb` (:496-520), `FOLLOW.rollback`/`restore` and the `Verb` members (:338-370), and the rollback sentence in the confirms (:150, :158, :170-181, :430). The confirm keeps "Current: {x}.";
- `text.ts:53-68,134,241-270`, the locales, `README.md:3,15`, `manifest.json:18`, `CHANGELOG.md`;
- tests `helpers.ts:228,241-285`, `install.test.ts:242-300`.

**Vestra Player AI** (`vestra.workshop`), replacing "install with rollback". **Recommended (Q3): Undo Install without host rollback.**

- After each committed install, the workshop keeps that `.vxp` in its data directory as `delivered/<id>.vxp`. Before that, it moves the one it already had to `delivered/<id>.previous.vxp`.
- **Undo Install**:
  - if the installed plugin's `source.kind` is `workshop` and a `.previous.vxp` exists: `validateArchive` + `install({source: workshop})` of that file. Same source, so it is an in-place install that keeps the instance and its data (`install_instance`, `package_manager.rs:1198-1205`);
  - otherwise (a first install, or one that replaced another source): `uninstall` keeping data, as today (`delivery.ts:235`).
- Edits:
  - `delivery.ts:1-5,13,38-47,53,75,129-140,160-161,213,220-241`;
  - `main.ts:278,480` (`canRollback` → `canUndo`);
  - `ai.slint:295,336-337,354,370-373,401-402,600`: `can-rollback` → `can-undo`, "Roll Back (Undo Install)" → "Undo Install", and the action word "rollback" → "reinstall";
  - `prompt.ts:69-71`, `worker/plugin-tools.ts:36`, `README.md:29,35`, locales;
  - tests `delivery.test.mjs:3,98-107`, `texts.mjs:32-39`, plus a new test for "upgrade then Undo reinstalls the kept `.vxp`".
- The host test `ui_types.rs:220-245` captures the AI window's properties (`"can-rollback"` at :233). The maintainer updates it in the same merge as the workshop rename.

## 5. Phases

| Phase | Owner | Work | Acceptance | Effort |
| --- | --- | --- | --- | --- |
| P0 | Coordinator | After `startup-seeding` (`6bf7e77`) merges: branch `plugin-management`. Baseline: §6.1 checks, test counts and line counts of store, plugins and workshop, and the stop-gap's Windows first-start and update timings | Baseline recorded here | 0.25 d |
| P1 | Maintainer | Host §2 (registry, in-place start, catalog migration, cleanup thread, rollback arm removed, `previousVersion: null`). `stage-plugins.mjs` schema 2, `packaging/smoke.mjs` and `check-licenses.mjs:412` path checks. SDK branch `1.5.16-rc.1` (deprecations, TestHost, docs). Migrate Plugin Manager. Host tests: migration of a 0.5.x catalog (built-in with current+previous → record; third-party previous dropped; idempotent; JSON parses with the 0.5.x structs); fresh profile; in-place start from a read-only copy; disable/enable a built-in across reopen; uninstall built-in → `unsupported`; reserved install → `reserved`; dev override; orphaned built-in → dormant; `packages.rollback` → `unsupported`; cleanup never runs on the tick | All §6.1 checks pass; Plugin Manager `check/build/test` pass and its source has no `rollback`/`previousVersion`; `npm run test:sdk` passes against `1.5.16-rc.1` | 2.5 d |
| P2 | Coordinator | Review against this doc; usage examples short | No P3 before pass | 0.25 d |
| P3 | Store author, workshop author (parallel), then info | §4 table | §5.3: `plugin -- check/build/test` pass; `grep -ri "rollback\|previousVersion\|roll back"` in the plugin source (tests and locales excluded) is empty; line counts reported | Store 0.5 d, workshop 1 d, info 0.1 d |
| P4 | Maintainer (+ infra maintainer) | SDK `1.5.17`: remove `rollback`, `previousVersion`, wire `rollback`, capability API entry; host drops `previousVersion` from `packages.list`; `host-agreement.test.ts:580`; `doc/architecture` updates; player depends on 1.5.17. Proposed: remove the built-in version guard (`src/scripts/check-builtin-versions.mjs`, `.github/scripts/builtin-base.sh`, `_static.yml`). In place, a changed built-in reaches users regardless of its version, which was the guard's only reason (#27); coordinator decides | `grep -rn "rollback\|previousVersion\|seeded"` over `src/host/src`, `src/runtime`, `src/plugins` (vendor excluded) and the SDK `src/` is empty apart from the migration reader; §6.1 in all three repos | 0.5 d |
| P5 | Coordinator | Windows GUI (below), plus a macOS spot check: `.app` first start and `codesign --verify --deep` | All pass; failed items retested after fixes | 0.5 d |
| P6 | Coordinator | Docs/status, commit, push, SDK tag `v1.5.17`, report | Merge is the user's call | — |

**Windows GUI checks (P5, coordinator)**

| # | Action | Expected |
| --- | --- | --- |
| G1 First start | Empty `%APPDATA%\Vestra`; start the installed build | Window responds at once, never "Not Responding"; all 18 built-ins `running`; `%APPDATA%\Vestra\packages\packages\` has no built-in directory; Plugin Manager shows the package folder under the install directory |
| G2 Update | On 0.5.x: disable one built-in, change a built-in setting (e.g. Hotkeys), install and update youtube once (so it has a previous version); then install the new build over it and start | No freeze; the disabled built-in stays disabled; the setting is kept; youtube runs with its data; the old built-in copies and previous-version directories disappear from `packages\packages\` within a minute; install during cleanup says "busy", then works |
| G3 Built-in disable/enable | Plugin Manager: disable a built-in, restart, enable | Stays disabled across restart; enable restores its menus; Uninstall… off with the hint; no Roll Back anywhere |
| G4 Third-party uninstall | Store: install youtube; uninstall keeping data; reinstall; uninstall with "Also delete its settings and data" | Reinstall reconnects the data; deleting removes it |
| G5 Store | Install, update, older version (per Q2), drop a `vestra.x` `.vxp` | Confirms have no rollback sentence; no Roll Back/Restore action; the reserved id is refused |
| G6 AI window | Generate, Install, Undo Install; Install v1, then v2, then Undo Install | First Undo uninstalls; second returns to v1 with its data (per Q3); texts say "Undo Install" |
| G7 Dev override | `--plugin src/plugins/info` | The development load runs instead of the built-in for that run |

## 6. Risks and questions

**Risks** (handled in the plan):

- A built-in that writes into its own directory fails on a read-only install. Mitigation: the P1 read-only test, plus G1/G2 on an Inno Setup install, not only the portable zip.
- The installer carries about 700 more loose files (workshop `vendor/` 344 files). Inno Setup and notarization handle this. The installer's install time should be watched in G2.
- If cleanup is interrupted, old directories stay until the next start. They are harmless (unreferenced).

**User rulings on Q1–Q3 (2026-10-26, binding; the coordinator recorded them):**

- **Q1: patch.** 1.5.16 deprecates, 1.5.17 removes, `apiVersion` stays `vestra/1`. community-workflow §1.6 step 2 is amended to match: SDK releases are patch bumps by default, removals included.
- **Q2: remove.** The store's Versions tab no longer installs an older version. The only way back to an older third-party version is to uninstall it and install a `.vxp` yourself.
- **Q3: superseded the same day. There is no Undo Install at all.** The AI window drops the Undo action. Plugin management has exactly three operations: **install, uninstall, disable/enable**. Built-ins cannot be uninstalled. The workshop keeps no previous `.vxp`.

Where §3–§5 assume the recommended Q2 (keep) or Q3 (A), these rulings override them; the implementers adjust.

**Questions for the user (answered above)**

- **Q1 (SDK version).** Removing `rollback` and `previousVersion` is breaking. Release it as **1.5.16 (deprecate) → 1.5.17 (remove), keeping `apiVersion: "vestra/1"`**, like the 1.5.8 `trial()` removal? Otherwise 2.0.0 + `vestra/2`, which refuses every existing third-party package. Recommended: patch, and amend §1.6 step 2 to match.
- **Q2 (older versions in the store).** The store's Versions tab can install an older version from the same repository in place (`installer.ts:162-181`). This is not rollback; it is like VS Code's "Install Specific Version". Keep it? Or remove it, so that the only way back is uninstall and then install a `.vxp`? Recommended: keep it.
- **Q3 (AI window Undo Install).** Choose between:
  - **A**, recommended: the workshop keeps the `.vxp` it last installed, and Undo after an upgrade reinstalls that file in place, keeping the data;
  - **B**: Undo only uninstalls (first installs), and there is no Undo after an upgrade.
