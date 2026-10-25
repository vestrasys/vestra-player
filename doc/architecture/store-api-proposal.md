# S4 core API change proposal: install instances, sources, built-in protection, manifest fields, Markdown component

Date: 2026-10-21. Author: API maintainer. Status: **confirmed (2026-10-21)**, see "Coordinator confirmation" at the end.
Based on: [Ecosystem plan](../product/ecosystem-plan.md) chapter 1 (1.2, 1.3, 1.4, 1.5a, 1.6) and chapter 4 S4; [Workflow](../process/community-workflow.md) §1, §1.5, §1.6, §4.3.

## 0. Conclusion

| # | Change | Location | Effect on the plugin API |
| --- | --- | --- | --- |
| 1 | The install registry is merged into the existing `packages/catalog.json`: the current install of each id carries an **install instance** (GUID, source, built-in flag, enabled, install time); an instance that is replaced or uninstalled (keeping data) becomes "dormant" and is reattached when the same source is installed again | Host | No change (plugins still get their directory from `VESTRA_PLUGIN_DATA_DIR`) |
| 2 | Data directory `plugin-data/<id>/` → `plugin-data/<GUID>/`; development loading uses `plugin-data/development/<id>/`; `secrets` is also isolated per GUID | Host | **One documented contract changes**: §3.4 "derive another plugin's data directory from `<data root>/<id>`" is withdrawn and replaced by the new field `PluginInfo.dataDirectory` (§7.3) |
| 3 | New in `packages`: `validateArchive(file)` (`.vxp`), `install({ source, replace })`, `PluginInfo` gains `builtin`, `source`, `dataDirectory` | Host + SDK | Additions only |
| 4 | Built-in plugins are identified by **source** (seeded from the installer); `uninstall` → `unsupported`, replacement from another source → `unsupported`; disable and rollback work as usual | Host | Only new error cases |
| 5 | Manifest gains optional display fields and `minPlayerVersion`; host and CLI validate with the same rule table; unknown fields are **still rejected** | Host + SDK CLI | Additions only |
| 6 | Core Markdown component: `MarkdownView` in `@vestra/markdown.slint` + the property value produced by `plugin.ui.markdown()`; the host parses with pulldown-cmark, remote images are downloaded and cached by the SDK in the plugin process | Host + SDK + widget library | Additions only, no new capability |
| 7 | SDK 1.1.0: types, TestHost, CLI (validation of new fields, hint for ids without a prefix, build includes icon/README, `pack` produces `.vxp`) | SDK repository | Additions only; the `pack` file extension changes (not plugin API) |
| 8 | Packaging: `Resources/plugins/` holds the built-in `.vxp` files and `builtin.json`; the host seeds them at startup | Host + `src/scripts/` | None |

Not in core: downloading `index.json`, signature verification, update checks, withdrawal handling, repository management, permission display. All of these live in the store plugin (S5).

---

## 1. Install instance GUID and install registry (1.5a)

### 1.1 Current state

| Code | Keyed by | Problem |
| --- | --- | --- |
| `package_store.rs` `Catalog.installed: BTreeMap<id, Installed>`, `disabled: BTreeSet<id>` | id | No source, no instance; the disabled set is stored separately |
| `plugins.rs` `load_manifest`: `data_root().join(&manifest.id)` | id | Two plugins with the same id share a data directory |
| `package_manager.rs` `uninstall`: `plugins::data_root().join(&id)` | id | Same as above |
| `system.rs` `secrets.*`: keychain service = manifest id | id | A replacement plugin with the same id can read the previous plugin's secrets |
| `main.rs` `load` (`--plugin`): data directory is also `<id>` | id | A development load shares data with the installed plugin of the same id |

### 1.2 Registry location and format

**No new file**; extend the existing `<app_data_dir>/packages/catalog.json` (`VESTRA_PACKAGE_STORE` can change the root). Reason: the instance must change in the **same atomic replacement** as install, rollback and recovery (`Store::publish` is already single-writer + temp-file rename); a separate file would mean handling two files that disagree.

```jsonc
{
  "installed": {
    "vestra.subtitles": {
      "revision": 3,
      "instance": {                          // new: the current install instance
        "guid": "3f0c8a8e-5b7e-4c1f-9d0a-2b6f1c7e9a41",
        "source": { "kind": "bundled" },     // source of built-in plugins after the revision (end of document)
        "builtin": true,
        "enabled": true,                     // replaces the old disabled set
        "installedAt": 1792540800000,        // first install of this instance, Unix milliseconds
        "seededVersion": "1.2.0"             // built-in only: version last seeded from the installer (§3.3)
      },
      "current": { "digest": "…", "manifest": { … }, "directory": "…/packages/<digest>" },
      "previous": { … }                      // previous version within the same instance, for rollback
    }
  },
  "dormant": {                               // new: dormant instances (only data remains)
    "9b1d…": { "id": "someone.viz", "source": { "kind": "link", "url": "…" },
               "version": "0.3.0", "installedAt": …, "retiredAt": … }
  },
  "pending": { … }, "operations": { … }, "lastCommit": { … }
}
```

| Field | Description |
| --- | --- |
| `guid` | UUID v4 generated by the host, lowercase with hyphens. Generated on a fresh install; kept on upgrade and rollback |
| `id`, version | These are the key of `installed` and `current.manifest.version`; not stored twice |
| `source` | The `PackageSource` of §2.3; the source of built-in plugins is `bundled` (the first draft used the official repository address; changed per "Coordinator revision" at the end) |
| `builtin` | Written by the host when seeding; plugins cannot set it |
| `enabled` | Written by `setEnabled`; the old `disabled` set is removed (an old catalog is converted from the set once on read) |
| `installedAt` | Time of the instance's first install |

Limit: `dormant` holds at most 256 entries; once full, new replacements/uninstalls still proceed, but **no reattachable registry entry is kept** (the data directory stays on disk; user data is not deleted automatically), and one line is written to the host log.

Not released, no migration: entries in an old catalog without `instance` get a new GUID when opened (empty data directory); the old `plugin-data/<id>/` is neither touched nor read.

### 1.3 Rules

| Case | Behavior |
| --- | --- |
| Fresh install of an id | New GUID, empty data directory |
| Same id, **same source** (same source identity per §2.3) | Upgrade: keeps GUID, data, `enabled`, `builtin` |
| Same id, **different source** | Without `replace: true` → `conflict`; with it → replace: the current instance becomes dormant (package files reclaimed, data kept), the new instance gets a new GUID and an empty directory |
| On replacement, the new source matches a dormant instance of the same id | "Switch back": reuse that GUID and data, delete the dormant record |
| The replacement target is a built-in plugin | `unsupported` (built-ins cannot be replaced), not even with `replace` |
| Replacement activation fails, is cancelled, or the player exits midway | The existing recovery flow restores `Installed` (including `instance`) as a whole; the data directory created for the new instance this time is deleted |
| `rollback` | Only within the same instance (`previous`); the GUID does not change. A new instance after replacement has no `previous` |
| `uninstall` without `removeData` | The current instance becomes dormant; reinstalling from the same source reattaches the data (equal to today's "uninstall keeps data, reinstall restores it" behavior) |
| `uninstall({ removeData: true })` | Deletes the instance's registry entry and data directory (after the process exits); no dormant record is kept |
| Running at the same time | Only one plugin per id runs: the existing `plugin already running` check in `load_package` is unchanged |

### 1.4 Directories and secrets

| Object | Now | Changed to |
| --- | --- | --- |
| Installed plugin data directory | `plugin-data/<id>/` | `plugin-data/<GUID>/` |
| Development load (`--plugin`) | `plugin-data/<id>/` | `plugin-data/development/<id>/` (independent of the installed plugin of the same id) |
| Trial | `plugin-data/<id>.trial.<owner>/`, deleted on stop | Unchanged |
| `secrets` keychain service | `<id>` | Installed: `vestra.plugin.<GUID>`; development load: `vestra.dev.<id>`; trial: unchanged (the service is the trial's own id `<id>.trial.<owner>`, already separate from the real install). Uninstalling with `removeData: true` also deletes the entries under that service (platforms that cannot do this return `unsupported` without blocking the uninstall; see §9.3, to be verified) |
| UI preferences (sidebar, floating window positions, binding overlay, options window page) | `<id>` | **Unchanged**: these are the user's preferences for that id's UI, not plugin private data; keeping them across instances is reasonable |

`VESTRA_PLUGIN_DATA_DIR` remains the only way a plugin gets its directory; plugin code does not change.

### 1.5 Code changes

| File | Change |
| --- | --- |
| `package_store.rs` | `Installed` gains `instance: Instance`; `Catalog` gains `dormant` and drops `disabled` (the old field is converted on read); `prepare_with_receipt` takes the target instance (keep on upgrade / new / switch back); written at `commit`; `uninstall(id, keep_data)` makes it dormant or deletes it; `set_enabled` writes `instance.enabled`; `validate_catalog` checks GUID format, the `dormant` limit, and that the same GUID does not appear in both `installed` and `dormant` |
| `package_manager.rs` | `packages.install` reads `source`, `replace`, decides the instance per §1.3 and returns `conflict` on conflict; `uninstall` rejects built-ins; `load_package` passes the instance to `Plugin::load_manifest`; `package_list` outputs `builtin`, `source`, `dataDirectory`; the data deletion path becomes the GUID directory |
| `plugins.rs` | `load_manifest` gets a new parameter `data_key: DataKey` (`Instance(guid)` / `Development(id)` / `Trial(owner)`) and builds the directory from it; `Plugin` remembers `data_key` for `secrets` |
| `main.rs` `load` | Development loading uses `DataKey::Development` |
| `system.rs` | The `secrets.*` service is taken from `data_key` |
| `Cargo.toml` | `uuid = { version = "=1.x", default-features = false, features = ["v4"] }` (already in the dependency tree, MIT/Apache-2.0) |

---

## 2. Additions to the `packages` interface (`plugins.manage`)

### 2.1 Existing interface (unchanged)

`validate(directory)`, `rollback(id)`, `list()`, `onChange()`, `setEnabled(id, enabled)`, `uninstall(id, { removeData })`; `PackageCandidate.trial()`, `install()`. Callers: `plugins/plugins/` in the plugin repository (`list`, `onChange`, `setEnabled`, `rollback`, `uninstall`), `plugins/workshop/` (`validate`, `trial`, `install`, `rollback`).

### 2.2 New signatures

```ts
interface Packages {
  // New: validate from a .vxp file (zip). Returns the same PackageCandidate as validate
  validateArchive(file: string): Promise<PackageCandidate>;
}
interface PackageCandidate {
  // The existing install() gains an optional parameter; without it, same as today (source is { kind: "file" })
  install(options?: { source?: PackageSource; replace?: boolean }): Promise<PackageChange>;
}
type PackageSource =
  | { kind: "repository"; url: string }   // full address of index.json (normalized by the store before passing)
  | { kind: "link"; url: string }         // plugin package link
  | { kind: "file" }                      // local package or directory dropped in or chosen
  | { kind: "workshop" };                 // generated by the plugin workshop
interface PluginInfo {
  // existing fields unchanged; new:
  builtin: boolean;                  // installed with the player: can be disabled, cannot be uninstalled or replaced from another source
  source: PackageSource | null;      // present when origin is "installed"; null for development loads and trials
  dataDirectory: string | null;      // its data directory; null when the player could not provide one
}
interface PackageManifest {
  // existing fields unchanged; new optional: name, author, punchline, description, tags, icon,
  // readme, changelog, sourceUrl, license, minPlayerVersion (§4)
}
```

- `validateArchive`: the host extracts to a temporary directory and goes through `Candidate::capture`; the checks are exactly the same as `validate` (manifest, digest, `node --check`, Slint compilation). The caller (the store) checks the `.vxp` sha256 itself before calling; the host does not know the expected value.
- The `source` of `install` goes into the wire `packages.install` parameters, so it is also part of the request fingerprint: the same `requestId` with a different source is rejected with `conflict` (existing deduplication rule).
- `PluginInfo.origin` gets **no new value** (built-ins are still `"installed"`, distinguished by `builtin`): adding a member to the union of an output type would break exhaustive `switch` statements written by authors at compile time, which is a hidden break.

### 2.3 Source identity (decides "upgrade" vs "replace")

| `kind` | Source identity | Reason |
| --- | --- | --- |
| `repository` | `url` (compared verbatim after WHATWG normalization) | The store "accepts updates only from the original source" (1.5a) |
| `link`, `file` | Both `manual` | A user manually installing a package with the same id intends to update; distinguishing by link would turn "use another version link" into a replacement and lose data |
| `workshop` | `workshop` | A user should be warned when a workshop-generated id collides with a manually installed plugin |

`url` is only recorded and compared; the host never accesses it. `url` must be `http(s)` and ≤ 2048 bytes, otherwise `invalid`. **This one needs a decision** (see the report): if link installs should also be distinguished by URL, change the table; the interface stays the same.

### 2.4 Replacement confirmation: the host rejects, the plugin confirms and retries with a flag

Choice: when the same id is already installed from **another source**, `install()` fails with `code: "conflict"` and does nothing; the plugin asks the user with `dialogs.confirm` and, on consent, calls `install({ source, replace: true })`.

| Option | Conclusion |
| --- | --- |
| **Host rejects + `replace` flag** (adopted) | The decision happens in the host, atomically with the commit (a plugin that calls `list()` first and then installs may see the state change in between); the confirmation UI stays in the plugin (wording and checkboxes are decided by the store/workshop; the host does not show dialogs on behalf of plugins, in line with "core contains no plugin business logic"); old calls without the flag never replace silently |
| Host shows its own confirmation dialog | The host would need to know the "store" and "workshop" wording and scenarios; does not work in headless mode |
| Rely only on the plugin checking `list()` first | Race condition; a plugin that forgets the check silently overwrites someone else's plugin |

If the plugin wants to show "where the installed one came from" before the dialog, it uses the `source` and `version` of that id in `list()`; the error object carries no extra data (`PluginError = Error & { code }` stays unchanged).

### 2.5 The GUID is not exposed

- Plugins address by id; there is only one instance per id at a time, so the GUID has no callable interface.
- What plugin management actually needs is only the data directory, already given by `dataDirectory`.
- No promise about the GUID, so the host can change the storage layout later; plugins also cannot persist or compare instances (for example, to track which sources a user has switched between).
- The GUID is visible in the `dataDirectory` path, but the documentation states that "the path format is not part of the contract".

### 2.6 Rollback

Unchanged. `rollback(id)` returns to the `previous` of the same instance; built-in plugins can be rolled back (§3.3 ensures seeding does not immediately overwrite a user's rollback). Going "back to the old plugin" across sources = reinstall from the old source (which reattaches the dormant instance's data).

---

## 3. Built-in plugins: can be disabled, not uninstalled

### 3.1 Identification

By **source**: only instances seeded by the host at startup from the installer's `Resources/plugins/builtin.json` carry `builtin: true`. A plugin cannot declare itself built-in through `install()`; the `builtin` flag still depends only on the source. Since 2026-10-24 the `vestra.` prefix is reserved for built-in plugins, and the host refuses to install or trial any other package whose id starts with `vestra.` ("Coordinator revision (2026-10-24)" at the end).

### 3.2 Behavior

| Operation | Built-in plugin |
| --- | --- |
| `setEnabled(id, false/true)` | As usual, persists across launches |
| `uninstall` | `code: "unsupported"`, message `built-in plugins can be disabled but not uninstalled` |
| `install` a new version from the same source (official repository) | Upgrade, keeps `builtin` |
| `install` from another source, with or without `replace` | `unsupported` (built-ins cannot be replaced) |
| `rollback` | As usual |

### 3.3 Seeding (at host startup)

`builtin.json` (generated by the packaging script):

```json
{ "schema": 1, "source": { "kind": "repository", "url": "https://…/index.json" },
  "plugins": [ { "file": "vestra.subtitles-1.2.0.vxp", "sha256": "…" } ] }
```

| Installed state | Action |
| --- | --- |
| This id is not installed | Install from the `.vxp` (new GUID, `builtin: true`, `seededVersion` = package version) |
| Installed and built-in | Upgrade to the packaged version only if the package version > `seededVersion` (if the user installed a higher version through the store, leave it); if the user rolled back, the same installer does not overwrite it again |
| Installed but from another source (the user sideloaded the same id before release) | Leave it, one log line; the built-in flag is not forced |

Seeding goes through the existing stage → prepare → activate → commit flow (failures are recovered per the existing rules and recorded in `failures`); no separate install path is written. `VESTRA_BUILTIN_PLUGINS=<directory>` can point to another seeding directory during development; a plain `cargo run` without that directory does not seed.

When a `--plugin` development load has the same id as an installed plugin: the development load wins, and the installed one does not start in this run (`state: "stopped"`). Check the existing order of `package_tick` and `load` first when implementing.

### 3.4 What the `plugins` plugin and the workshop need to know

- Plugin management: when `builtin` is true, disable "Uninstall…" with the hover text "Built-in plugins can only be disabled"; details show the source (official repository / third-party repository address / link / local file / plugin workshop). `uninstall` must still handle `unsupported` (the list may be out of date).
- Workshop: when a generated id collides with a built-in plugin, `install` gets `unsupported`; prompt the user to change the id. Since 2026-10-24, when the generated id starts with `vestra.`, `validate`, `trial` and `install` get `reserved` (revision at the end).

---

## 4. New manifest fields

### 4.1 Fields and validation (one table for host and CLI)

| Field | Type | Rule |
| --- | --- | --- |
| `name` | string | 1–64 characters (Unicode scalars), no control characters, no leading/trailing whitespace |
| `author` | string | 1–64 characters, same as above |
| `punchline` | string | 1–120 characters, single line |
| `description` | string | 1–2000 characters, line breaks allowed, plain text |
| `tags` | string[] | At most 8, each 1–24 characters, single line, deduplicated case-insensitively |
| `icon` | string | Relative path inside the package (same path rules as `entry`: no `..`, `\`, absolute paths, symbolic links); extension `.png`/`.jpg`/`.jpeg`; the file must be in the package; ≤ 512 KB; PNG/JPEG by content, side length ≤ 1024 (256×256 square recommended) |
| `readme` | string | Relative path inside the package, `.md`, UTF-8, ≤ 256 KB. If omitted and the package has `README.md`, that file is used |
| `changelog` | string | Same as `readme`; if omitted and there is `CHANGELOG.md`, that file is used |
| `sourceUrl` | string | `https:` address, parses successfully per WHATWG, ≤ 2048 bytes |
| `license` | string | 1–64 characters (an SPDX expression is recommended; not checked against the SPDX list) |
| `minPlayerVersion` | string | Strict `x.y.z`, same as `version` (§6) |

- An explicitly given `icon` / `readme` / `changelog` whose file is missing → rejected; a missing default `README.md` / `CHANGELOG.md` → no error.
- The fields are for display only; apart from validation the host does not use them (except `minPlayerVersion`). The host does not render the README or show the icon.
- Development loads (`--plugin`) are validated the same way; paths are relative to the source directory.

### 4.2 Unknown fields: still rejected

Today both `plugins::Manifest` and `package_stage::PackageManifest` are `deny_unknown_fields`, and the reference documentation also states they are rejected. Keep it:

- A misspelled field (`capabilties`) shows up immediately instead of silently doing nothing;
- When fields are added later, authors also write `minPlayerVersion`, so the store will not push the plugin to old players;
- When sideloaded into an old player, the host **reads `minPlayerVersion` first** and then parses strictly, so the error is "requires Vestra Player ≥ x.y.z" rather than "unknown field".

Implementation hint: merge the two manifest structs into one module (suggested `src/host/src/manifest.rs`), with one set of fields and two validation modes (development/package). serde's `flatten` cannot be combined with `deny_unknown_fields`, so do not reuse fields through `flatten`.

### 4.3 id rules

Unchanged (decided in 1.5a): non-empty, ≤ 128 bytes, ASCII letters and digits plus `.-_`. The host does not enforce a prefix. (Revision 2026-10-24: `vestra` and ids starting with `vestra.` are reserved for built-in plugins; see the end.)

---

## 5. Core Markdown display component (1.6 item 10)

### 5.1 Shape: a new widget-library control + the host parses into a model

```slint
import { MarkdownView } from "@vestra/markdown.slint";
export component Readme inherits Window {
  in property <[MarkdownBlock]> readme;      // the plugin writes the result of plugin.ui.markdown()
  callback open-link(string);
  MarkdownView { blocks: root.readme; link-clicked(url) => { root.open-link(url); } }
}
```

```ts
const doc = await plugin.ui.markdown(text, { base: readmeUrl });   // no capability needed
window.set({ readme: doc });
window.on("openLink", (url: string) => plugin.system.openUrl(url));
```

Mapping to the existing architecture:

| Step | Approach | Basis |
| --- | --- | --- |
| Passing the value over the wire | `MarkdownRef` is the same kind of thing as `ImageRef`: it can only be the whole value of a property; the wire form `{ "$vestraMarkdown": { text, base, images }, "ref": n }` is an internal detail | `$vestraImage` in `ui_image.rs`; add one branch to `Host::prop_value` |
| Parsing | The host parses in `prop_value` into a Slint model of `[MarkdownBlock]` (`slint_interpreter::Value::Model` of Struct), building images directly as `Value::Image` | Images cannot enter a struct through JSON (limitation in reference §6.1); building the model on the host side avoids this |
| Image loading | Reuse `ui_image::Images`: the same path roots (plugin package and data directory), formats, 16 MB / 4096² limit, 64 MB cache per plugin | `ui_image.rs` |
| Rendering | `src/host/ui/vestra/markdown.slint` (new library file) exports `MarkdownView` and `struct MarkdownBlock`; `ScrollView` + `for block in blocks` draws headings, paragraphs, list items, code blocks, quotes, images, tables and rules by `kind` | Widget library mechanism unchanged; add the new file to `SLINT_FILES` in `sync-sdk-slint.mjs` |
| Links | `callback link-clicked(string)` of `MarkdownView` gives an absolute address (relative links are resolved against `base`; `#anchor` is passed as-is); whether to open it is up to the plugin (usually `system.openUrl`) | Existing `system.openUrl` |

It goes in a separate library file rather than `widgets.slint`: the control is large, and plugins that do not use it do not load it at compile time.

### 5.2 Supported subset

| Element | Rendering |
| --- | --- |
| Headings 1–6 | Font size graded by `Theme` |
| Paragraphs, emphasis, bold, strikethrough, inline code, links | See §5.4 |
| Unordered / ordered lists, task lists | Nesting depth ≤ 4 (deeper levels flattened to level 4); task items show ☐ / ☑ |
| Code blocks | `Theme.font-mono`, wrapped, no syntax highlighting |
| Quotes | Vertical bar on the left, nesting ≤ 4 |
| Images | Local or downloaded PNG/JPEG; scaled down to the width, never up; failures and unsupported formats (SVG, GIF, WebP) show the alt text |
| Tables (GFM) | Cells are plain text (inline styles removed), equal column widths, ≤ 16 columns × 500 rows |
| Horizontal rules | Yes |
| Embedded HTML (block and inline) | **Dropped**; the source is not shown |
| Footnotes, math, definition lists, front matter | Not enabled; treated as normal text |

Limits: `text` ≤ 256 KB (wire messages 1 MiB), blocks ≤ 4000, images ≤ 64; the excess is truncated and a final block "Content too long, truncated" is added.

### 5.3 Remote images: downloaded by the SDK in the plugin process, cached in the plugin data directory

| Option | Conclusion |
| --- | --- |
| **SDK downloads** (adopted) | Node has built-in `fetch`, cross-platform, no new dependency; the download belongs to the calling plugin, and its resources are reclaimed together with disabling; the cache lives in that plugin's own data directory, so the `ui_image` path roots need not be widened |
| Host downloads | The Rust side would need a new HTTP + TLS stack (ureq/rustls and per-platform certificates), widening the core and the cross-platform surface for a single display feature |

Flow: `ui.markdown(text, { base })` → the SDK sends `ui.markdownScan` (the host parses and returns the absolute addresses of the remote images needed, at most 64) → the SDK downloads to `<dataDirectory>/.vestra-cache/markdown/<sha256(url)>` → builds the wire value, with `images` as "address → local path".

| Rule | Value |
| --- | --- |
| Protocol | `https:` only; `http:` and others show the alt text |
| Per image | ≤ 4 MB, only PNG/JPEG by content, 10-second timeout, ≤ 5 redirects, no cookies |
| Concurrency | 4 per plugin |
| Cache | 32 MB per plugin, least-recently-used eviction; the same address is not downloaded again within 24 hours |
| Failure | Does not make `markdown()` fail; the image shows its alt text |
| `options.remoteImages: false` | No downloads |

### 5.4 Inline styles: use Slint's `StyledText`, needs verification first (pending decision)

Slint 1.16.1 already has a `StyledText` element (bold, italic, strikethrough, inline code, links and `link-clicked`), but the corresponding `Value::StyledText` in `slint-interpreter` is marked `#[doc(hidden)]` ("shown once StyledText is public"), so the host would have to use an unpublished interface to construct it.

- First implementation step is probe M0: in a plugin window, use the interpreter to show a `StyledText` built by the host, including Chinese text and link clicks.
- Pass: paragraphs, list items, quotes and headings use `StyledText`; the construction code is kept in one function, Slint is pinned to `=1.16.1`, and it must be retested on every Slint upgrade.
- Fail: fall back to plain text (inline markup removed, inline code keeps its text); links in a paragraph are listed below that paragraph as clickable lines. The interface stays the same; rendering can be upgraded later.

### 5.5 Rust parsing library

`pulldown-cmark`: MIT license, CommonMark + GFM tables/strikethrough/task lists; **already in the dependency tree** (Slint 1.16.1 depends on 0.13.4); the direct dependency is pinned to the same version `=0.13.4` with `default-features = false`, bringing in no new code. First-party code stays `forbid(unsafe_code)`. Parsing is pure computation and system-independent, which satisfies the §1.5 cross-platform rule.

### 5.6 Capabilities

No new capability. Same as `ui.image`: it is only a kind of property value; the `ui.window` / `ui.mount` / `ui.settings` needed to show it already limit where it can appear; downloads run in the plugin's own process, and capabilities never restricted network access anyway.

---

## 6. `minPlayerVersion`

- **Source of the player version number**: the `version` in the host `Cargo.toml` (`env!("CARGO_PKG_VERSION")`), i.e. the `plugin.app.version` that plugins see. At packaging time `Info.plist` / the Windows version resource are generated from it; no second copy is maintained. Suggestion: when S4 ships, bump the host from `0.1.0` to `0.2.0` as the first version supporting the new API.
- **Semantics**: a plugin can load only if the player version (only the three numeric parts `MAJOR.MINOR.PATCH` are compared; pre-release suffixes are ignored) ≥ `minPlayerVersion`. It expresses "uses an API **added** in a certain player version"; `apiVersion` still expresses breaking major versions. There is no `maxPlayerVersion`.
- **Check points**: `validate` / `validateArchive` → `code: "versionMismatch"` (the host already has this code; add it to reference §13), message `requires Vestra Player ≥ x.y.z`; development load → load fails with the same message; an installed plugin on a downgraded player → does not start, `state: "failed"`, `error` as above; built-in seeding skips it and logs.
- **SDK version table**: the SDK README gets a table "APIs added in SDK 1.1 require player ≥ 0.2.0". The CLI does not infer it automatically (that would require analyzing which APIs are used; not worth it).
- Each version entry in `index.json` should get `minPlayerVersion` so the store can filter on it; this belongs to S3/S5, see §8.

---

## 7. SDK 1.1.0 scope

### 7.1 Contents

| Part | Contents |
| --- | --- |
| Types (`types.ts`) | All of §2.2; new optional fields on `PackageManifest`; `plugin.ui.markdown(text, { base?, remoteImages? }): Promise<MarkdownRef>`, the `MarkdownRef` type; `PackageSource` |
| Implementation | `packages.ts`: `validateArchive`, `install(options)`, `info()` carries the new fields; `ui.ts`: `markdown()` and the download cache (Node built-in modules only, keeping zero runtime dependencies) |
| wire (internal) | `packages.validateArchive`; `packages.install` parameters gain `source`, `replace`; `packages.list` / `packages.changed` entries gain fields; `ui.markdownScan` |
| TestHost | New fields on `host.packages` entries are **all optional** (defaults `builtin: false`, `source: null`, `dataDirectory: null`), so existing tests need no change; `uninstall` of a built-in entry → `unsupported`; new `host.candidates` (path → manifest) supports `validate` / `validateArchive` / `install`, simulating `conflict` and `replace` per §1.3, and `packageCalls` records `install`; the wire value of `ui.markdown` can be read back as `{ text, base, images }` with `host.markdown(value)`; remote images do not go to the network and are provided by `host.remoteImages` (address → local path or `null`) |
| CLI `check` | Validates all new fields per §4.1 (the rule table is exported as constants from `./runtime` and compared by a host agreement test); prints a **hint** when the id has no `.` prefix (such as "publishing to the official repository requires `<GitHub username>.<plugin name>`"), exit code 0; unknown manifest fields are errors (consistent with the host) |
| CLI `build` | Copies the `icon`, `readme`, `changelog` referenced by the manifest (and the default `README.md`, `CHANGELOG.md`) into `dist/`. Today `.md` files are always skipped; without this change the README does not get into the package |
| CLI `pack` | Produces `<id>-<version>.vxp` (same content as today's deterministic zip, only the extension changes) |
| Documentation | README, CHANGELOG, TypeDoc; the version table gets `1.1 ↔ player ≥ 0.2.0` |

### 7.2 Deferred: `npm create vestra-plugin`

Not in this round. Reasons: it is a separate npm package (`create-vestra-plugin`) that needs its own release process; the template contents (`name`, `author`, `icon`, README skeleton, id prefix) only become stable once this proposal is final; S4/S5 do not depend on it. Suggested to do it after S5, together with the plugin repository's contribution guide.

### 7.3 Compatibility review

Everything is additive: new methods, optional parameters, new properties on output interfaces, a new library file, new wire methods. Hidden breaks checked item by item:

| Point | Breaking? | Handling |
| --- | --- | --- |
| **Reference §3.4 "derive another plugin's data directory from `<data root>/<id>`"** | **Yes** (a rule promised by the documentation stops holding) | Unavoidable: the purpose of GUID directories is to stop locating by id. Replacement: the same version adds `PluginInfo.dataDirectory`. The only user is plugin management (`dataDirectory(id)` in `manager.ts`), migrated in this round; the reference and the SDK comment (the "`<data root>/<id>`" on `PluginInfo.directory`) are updated together. Not released |
| Data and secret locations change | Not for plugin code; yes for user data (development-time data and logins are lost once) | 1.5a decided no migration; notify youtube, bilibili (login secrets) |
| `install()` without parameters meets the same id from another source → `conflict` | New error semantics | "Another source" does not exist today (no store, no link install), so existing callers see no behavior change; the workshop changes this round to pass `source: workshop` and handle `conflict` |
| `uninstall` gains an `unsupported` case, `install` gains `unsupported` (built-in) | No | Both methods already have `unsupported`; callers must already handle it |
| `PluginInfo.origin` does not get `"builtin"` | — | Deliberately avoided (§2.2) |
| TestHost entry type gains fields | No | New fields are optional |
| `pack` output `.zip` → `.vxp` | CLI behavior change, not plugin API | S0 already noted the format was provisional; stated in the SDK CHANGELOG; notify S3 |
| `apiVersion` | Unchanged `vestra/1` | |

---

## 8. Impact and notification list

| Party | Needs to know | Needs to do |
| --- | --- | --- |
| Plugin management / store (`plugins/plugins/` in the plugin repository) | `dataDirectory`, `builtin`, `source`; `validateArchive`; `install({ source, replace })` and `conflict`; the Markdown component | Read the data directory from `info.dataDirectory`; disable uninstall for built-ins; show the source; use the new interfaces from S5 on |
| Plugin workshop (`plugins/workshop/`) | `install` may return `conflict` (id collides with a manual or repository install) or `unsupported` (id collides with a built-in) | Pass `source: { kind: "workshop" }`; on `conflict`, confirm and then `replace: true`; on `unsupported`, prompt to change the id |
| youtube, bilibili | `secrets` are now stored per instance; existing logins must sign in again once | No code change |
| All plugin authors | The data directory location changes (development load in `plugin-data/development/<id>/`); `build` packs `README.md`, and the README will be shown on the store detail page; new optional display fields; write `minPlayerVersion` when using new 1.1 APIs | Before entering the official repository, add `name`, `author`, `punchline`, `icon`, and rewrite the README for users |
| S3 (plugin repository CI) | `pack` produces `.vxp`; `index.json` version entries should get `minPlayerVersion`; the player side maintains the built-in plugin list | Upload `.vxp` files to Releases |
| Coordinator | §9.3 real-device verification points; questions needing a decision (report) | |
| Documentation (maintainer) | Reference §2, §3.4, §6.1 (new Markdown subsection), §12, §13 (`versionMismatch`), §17 change notes; the architecture overview code map | Update along with the implementation |

---

## 9. Implementation split, order and tests

### 9.1 Order (maintainer alone; parallelizable items in parentheses)

| Step | Contents | Depends on |
| --- | --- | --- |
| A | Host: `manifest.rs` merges the two manifest structs, validation of new fields and `minPlayerVersion` | None |
| B | Host: catalog `instance` / `dormant`, GUID data directories, development load directory, per-instance `secrets`, built-in protection, `source`/`replace`, new `packages.list` fields | A |
| C | Host: `validateArchive` (extraction, see below) | A (parallel with B) |
| D | Host: `builtin.json` seeding + `src/scripts/stage-plugins.mjs` (takes `.vxp` files from the plugin repository Releases or a local directory, checks sha256 against the list) | B, C; use local `.vxp` files until S3 has Releases |
| M0→M | Markdown: probe M0 (§5.4) → parsing and model → `markdown.slint` → `prop_value` branch → `ui.markdownScan` | Parallel with A–D |
| S | SDK 1.1.0: ship types and TestHost first (`1.1.0-rc.1` can let S5 authors start writing code early), then the implementation and CLI | The wire protocol is fixed along with B, C, M |
| P | Player upgrades the SDK dependency, host-agreement tests add the rule-table comparison, documentation; notify plugin management and workshop authors to migrate | S |

`.vxp` extraction: suggested to use the `zip` crate (MIT, `default-features = false`, deflate only, exact version pinned) rather than hand-written zip parsing. This is security-sensitive code that handles external input; a mature library is safer. Limits match `package_stage` and are enforced **during extraction**: archive ≤ 64 MB; entries ≤ 1024; each file ≤ 32 MB after extraction, total ≤ 64 MB (against zip bombs); only stored/deflate, no encryption; reject absolute paths, `..`, `\`, symbolic links (external attributes), case-insensitive duplicate names; `manifest.json` must be at the root. The new dependency counts as "introduces license obligations", see the report.

### 9.2 Tests

| Layer | Contents |
| --- | --- |
| Host unit tests `package_store` | Fresh install generates a GUID; same-source upgrade keeps it; different source without `replace` is rejected, with it replaced and made dormant; switch back reuses the GUID; built-ins cannot be uninstalled or replaced; failed replacement restores `instance`; `dormant` limit; `enabled` across reopen; reading an old catalog (with `disabled`, without `instance`) |
| Host unit tests `plugins` / `system` | Data directory per `DataKey` lands in GUID / `development/<id>` / the trial temp directory; `removeData` deletes the GUID directory; the `secrets` service follows the instance |
| Host unit tests manifest | Boundaries of each field (length, control characters, path escapes, icon format and size, missing default README is no error, missing explicit one is an error, URL protocol); unknown fields rejected; `minPlayerVersion` errors before strict parsing |
| Host unit tests `.vxp` | A normal package has the same digest as the `pack` output; `..`, absolute paths, symbolic links, duplicate names (case), encryption, over-limit, bombs (high compression ratio) are all rejected |
| Host unit tests Markdown | Model output for each block type; HTML dropped; relative links resolved against `base`; truncation over limits; image paths out of bounds go through the `ui_image` rejection |
| Host unit tests seeding | First seeding; upgrade only for a higher version; after a rollback, not overwritten by the same version; skip when another source already holds the id; skip on sha256 mismatch |
| SDK unit tests | Wire parameters of `validateArchive` and `install(options)`; new `info()` fields; markdown download: protocol, size, type, cache eviction, failures do not throw (with a local HTTP server, no external network) |
| TestHost | Conflict and replacement simulation; old-shape `host.packages` entries still work; `host.markdown()` |
| CLI | `check` errors for new fields and the no-prefix hint; `build` includes README/icon; `pack` produces `.vxp` deterministically |
| Agreement | `host-agreement.test.ts` compares the manifest rule constants; `sync-sdk-slint` covers `markdown.slint` |
| Full §6.1 suite | Host fmt/clippy/test; `src/` check, boundaries, test:sdk, plugins:build, each plugin's tests; SDK repository CI; portability (Windows target compile) |

### 9.3 Real-device verification by the coordinator

1. Markdown: show in one window a document with Chinese text, headings, lists, code blocks, quotes, tables, local and remote images, and links; clicking a link opens the browser; scrolling is smooth; resizing the window rewraps the text. The actual result of the StyledText route (or the fallback) in the GUI.
2. Install by dragging in / choosing a `.vxp` (through plugin management or a temporary test plugin), hot-enabled after install.
3. Replacement flow: install the same id from two sources; confirmation dialog, replacement, and data still present after switching back.
4. Built-in protection: in the packaged layout (`Resources/plugins`), the first launch seeds all built-in plugins; in plugin management, "Uninstall" is unavailable for built-ins, they can be disabled, and stay disabled after restart.
5. Data isolation: after replacement, the new plugin cannot see the old plugin's data directory contents or secrets (once each for macOS keychain and Windows Credential Manager).
6. Windows: `.vxp` paths and case, GUID directories, secret service names.

---

## 10. Deliberately not done

| Temptation | Reason for not doing it |
| --- | --- |
| Host downloads `index.json`, verifies signatures, checks for updates, handles withdrawals | 1.4 placed these in the store plugin; signatures "may move into core later" |
| Built-in HTTP client in the host (for remote images) | Brings in a TLS stack and platform certificate differences for a single display feature; Node `fetch` is enough (§5.3) |
| `PluginInfo` carrying display fields (name, icon…) | Plugins can read `info.directory/manifest.json` directly (already validated by the host); one less projection to maintain |
| Exposing the GUID, `installedAt` | No caller needs them (§2.5); add them via a §4 request when needed |
| Listing and cleanup interface for dormant instances | First observe whether many leftovers really occur; when needed, design it separately as a plugin management request. User data is not deleted automatically for now |
| Widening `ui.image` so the store can show icons inside other plugin packages | The store can copy the file into its own data directory and show it there; no cross-plugin read path opened for a single plugin |
| `PluginInfo.origin` gaining `"builtin"` | Adding a member to an output union is a hidden break (§2.2) |
| Host enforcing an id prefix | Dropped in 1.5a (reserving `vestra.` was changed to "do it" on 2026-10-24, see the end) |
| Ignoring unknown manifest fields instead | Loses misspelling detection; `minPlayerVersion` already solves forward compatibility (§4.2) |
| `maxPlayerVersion`, version ranges | No need; `apiVersion` handles breaking changes |
| Markdown: embedded HTML, syntax highlighting, SVG/GIF/WebP, footnotes, math, editing, table-of-contents navigation | 1.6 item 10 asks only for the common subset; each item needs extra parsing or decoding code |
| A Markdown-specific capability name | It is only a property value; where it can be shown is already constrained by capabilities (§5.6) |
| `.vxp` file association (double-click to open), dropping onto the main window to install directly | Needs platform registration and an install confirmation UI; drop-to-install is done by the store with the existing `ui.drop` |
| `npm create vestra-plugin`, `vestra-repo build` | The former is deferred until after S5 (§7.2); the latter belongs to S3 |
| Plugin sandbox, restricting network and files by capability | 1.6 item 4: later |
| Automatic updates (without user confirmation) | 1.6 item 5: manual confirmation |

## Coordinator confirmation (2026-10-21)

- The proposal is approved as a whole and is implemented in stage 3 in the order of §9.
- Link installs and local file installs count as the same source (sideloading), as proposed.
- Markdown inline styles start with probe M0; if `StyledText` is not usable in the interpreter, falling back to plain text is the maintainer's decision, without asking again.
- Secrets are isolated per instance, and existing YouTube and Bilibili logins become invalid once: accepted (not released).
- Adding the `zip` crate (MIT): the user has agreed. The release package must include its license text, collected together with the other third-party licenses.

## Coordinator revision (2026-10-21, built-in plugins stay in the main repository)

The user decided that built-in plugins stay in the player repository (revision of ecosystem plan 1.6 item 6); only `youtube` and `bilibili` move to the plugin repository and are installed through the store. Effect on this proposal:

- The source of built-in plugins is the **installer** (`source.kind = "bundled"`), not the official repository address; `builtin.json` and the `.vxp` files are built by the packaging script from `src/plugins/`.
- The first store version **neither updates nor replaces** built-in plugins: any `install` for a built-in id (whatever the source, with or without `replace`) returns `unsupported`; only seeding (a higher version in a new installer) upgrades built-in plugins. The row "`install` a new version from the same source (official repository) → upgrade" in the §3 table is withdrawn; the case "if the user installed a higher version through the store, leave it" in §3.3 no longer occurs.
- Rollback and disabling work as usual.

## Coordinator revision (2026-10-24, the `vestra.` prefix is reserved for built-in plugins)

User ruling ("Plugin categories" at the start of the ecosystem plan): the only official plugins are the built-in plugins shipped with the player's main repository; their ids use `vestra.<name>`, and this prefix is reserved for them. All other plugins are third-party plugins, including those in the official plugin repository `plugins/`; they are only hosted in the official repository with stricter review, and their ids follow `<GitHub user or organization>.<name>`. youtube and bilibili become `zysilm.youtube` and `zysilm.bilibili` (author zysilm). The player has not been released, so there is no migration. This overturns "no id gets special treatment" and "`vestra.` is not reserved" in §4.3 and §10. What the host does:

- `packages.validate`, `validateArchive`, `trial` and `install` reject packages whose id is a reserved id (case-insensitive, equal to `vestra` or starting with `vestra.`), with error code `reserved` and the message "“<id>”: ids starting with vestra. are reserved for the player's built-in plugins and cannot be installed". Store installs, sideloading (dropping a `.vxp`, installing from a file, installing from a link), and installs and trials by the plugin workshop and AI all go through these methods.
- `packages.rollback` for a reserved id works only on a built-in instance (that is, rolling back a built-in plugin, as before).
- Built-in plugins come only from the installer's built-in plugin set (`builtin.json`, source `bundled` when seeded); this is the only way a reserved id gets into the install store.
- At startup, leftover non-built-in plugins with reserved ids in the install store (installed before this rule) do not start; `packages.list` shows them as `failed` with the sentence above as `error`; the user can uninstall them.
- `--plugin DIR` development loads with a reserved id are still allowed (built-in plugin authors develop with `--plugin`); the host prints a warning on stderr. Development loads are never written to the install store; their data is in `plugin-data/development/<id>/`.

## Implementation record (2026-10-21, stage 3, API maintainer)

Implemented items 1–8 of §0 in the order of §9, plus the coordinator revision (built-in plugin source `bundled`, any `install` for a built-in id is `unsupported`). The player is in the `rebuild/store-core` worktree, the SDK on the `store-1.1` branch of `vestra-player-sdk` (commit `453088e`, version 1.1.0, not tagged, not pushed). The host version is bumped to **0.2.0** (`Cargo.toml`), which is the `minPlayerVersion` required by the interfaces added in SDK 1.1.

### Host

| File | Contents |
| --- | --- |
| `manifest.rs` (new) | The `Manifest` shared by development loads and managed packages (`plugins::Manifest` and `package_stage::PackageManifest` are both aliases of it) and the rule table (§4.1); `parse` reads `minPlayerVersion` first and then parses strictly; the `NeedsPlayer` error is mapped to `versionMismatch` in RPC |
| `package_store.rs` | `Installed.instance` (GUID, `source`, `builtin`, `installedAt`, `seededVersion`), `Catalog.dormant` (≤ 256), `Recovery.instance`; at commit, upgrade/replace is decided by GUID (replace: the old instance becomes dormant, `previous` is cleared; switch back: taken back from dormant); `uninstall(keep_data)`; an old catalog gets instances when opened |
| `package_manager.rs` | `install` reads `source`/`replace` to decide the instance (`install_instance`, §1.3/§2.3); `packages.validateArchive`; `uninstall` rejects built-ins; new `packages.list` fields; a development load with the same id takes priority; `seed_builtins` seeding |
| `package_archive.rs` (new) | `.vxp` extraction (`zip` 9.0.5, deflate only); limits enforced during extraction |
| `plugins.rs` | `DataKey` (`Instance(guid)` / `Development(id)` / `Trial`) decides the data directory and the secret service name |
| `system.rs` | The service name of `secrets.*` is taken from `DataKey` |
| `ui_markdown.rs` (new), `ui/vestra/markdown.slint` (new) | pulldown-cmark 0.13.4 parses into the `[MarkdownBlock]` model; `prop_value` in `ui_image.rs` recognizes `$vestraMarkdown`; `ui.markdownScan` (no capability) returns the remote images to download |
| `ui_values.rs` | `reconcile` replaces the whole value when it meets non-JSON values (images and styled text in Markdown rows) instead of reporting an error |

**M0 probe conclusion: use `StyledText`**. `slint::private_unstable_api::re_exports::{parse_markdown, string_to_styled_text}` constructs `Value::StyledText` (that variant is `doc(hidden)` in the interpreter); the `StyledText` element in plugin `.slint` files and the struct field `styled-text` both compile and render, and `link-clicked` works (headless rendering test in `ui_markdown`: text, image pixels, link click callback). To keep user text out of Slint's Markdown parsing, the format string contains only markup we generate and the placeholder U+E541, and all text is passed as placeholder arguments; `parse_markdown` is wrapped in `catch_unwind` and falls back to plain text on failure. Slint is pinned to `=1.16.1`; this spot must be retested on every Slint upgrade (`Inline::styled` in `ui_markdown.rs` and the rendering test).

**Implementation choices that differ from the proposal**

- The enabled state is still stored in the original `disabled` set (by id, applying only to the current instance) and was not moved into `instance.enabled`: same semantics, smaller change.
- Built-in plugin seeding **commits directly** at startup without waiting for activation to succeed; the startup flow then starts them like any other installed plugin, and a failed start shows as `failed`. If seeding waited for activation, built-in plugins would disappear from the list in headless mode or after a failed start.
- The trial data directory and secret service name are unchanged (already isolated by `<id>.trial.<owner>`).
- Extraction errors in `validateArchive` are `invalid` (errors from directory `validate` are still the original `host`).
- Uninstalling deletes the data, but the instance's entries in the system keychain are not deleted (the entries under a service cannot be enumerated); the new instance has a different service name and cannot read them.

### Packaging

`src/scripts/stage-plugins.mjs <target>/plugins`: runs `vestra-plugin pack` for each directory with a `manifest.json` under `src/plugins/`, producing `<id>-<version>.vxp` and `builtin.json` (`{ schema: 1, plugins: [{ id, version, file, sha256 }] }`; `.zip` files produced by SDK 1.0 are renamed automatically). The host reads from `$VESTRA_BUILTIN_PLUGINS`, otherwise `<exe>/../../Resources/plugins`; without `builtin.json` there is no seeding (development runs). youtube and bilibili are deleted by another worktree and so are naturally no longer packaged.

**New dependencies and licenses**: `zip` 9.0.5 (MIT; the only crates newly brought into the dependency tree are `typed-path` and, on Unix, `nix` 0.31, all MIT or MIT/Apache-2.0; the others, `flate2`, `crc32fast`, `indexmap`, `memchr`, were already in the tree), `uuid` 1.28.0 and `pulldown-cmark` 0.13.4 (both already in the dependency tree). The repository currently has no place that collects third-party license texts; including licenses in the release package needs to be handled in the packaging task.

### SDK 1.1.0

Types, `packages.validateArchive`, `install(options)`, new `PluginInfo` fields, `ui.markdown` (`src/plugin/markdown.ts`: Node `fetch` downloads only `https:`, follows ≤ 5 redirects manually, 4 MB per image, 10 seconds, concurrency 4, cache 32 MB/24 hours), `src/manifest.ts` (the CLI's rule table; `MANIFEST_LIMITS`, `MANIFEST_FIELDS`, `MARKDOWN_LIMITS` are given to the player's agreement tests through `./runtime`), TestHost, CLI, `slint/vestra/markdown.slint`, README / Getting started / `CHANGELOG.md`. All additive; compatibility review as in §7.3. `npm create vestra-plugin` is deferred per §7.2.

### Documentation and examples

`doc/plugins/reference.md` (§2, §3.4, §6.1, §12, §13, §15, §17), `doc/plugins/store-changes.md` (notice and examples for plugin maintainers), `doc/plugins/README.md`, `doc/architecture/README.md`; example plugin `src/examples/markdown/` (Markdown window, display fields, `minPlayerVersion`).

### Checks (actually run this time)

- Host (`CARGO_TARGET_DIR=~/.cache/vestra-target/core`): `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings` pass; `cargo test --locked` 287/287 (baseline 263, 24 new). `npm run portability`: clean, the host compiles for the Windows target.
- SDK repository: `npm ci`, `check`, `format:check`, `build`, `npm test` 111/111 (baseline 105), `npm pack --dry-run` (83 files), `npm run docs` (warnings as errors) pass.
- Player (temporary `npm install <SDK repository>`): `check`, `format:check`, `boundaries` pass; `test:sdk` 11/11; `plugins:build` passes; 19 plugin test suites 552/552, identical to the baseline one by one; examples bookmark 1, core-ui 2, panel-menu 1, markdown 1. Before delivery, `src/package.json` and `package-lock.json` were restored to `^1.0.0` (byte-identical to before the commit).
- Headless end-to-end (`vestra-host --headless`, temporary install store and data root): first seeding of 19 built-in plugins (source `bundled`, each with its own GUID directory); a second launch does not seed again; with 2 plugins at higher versions in a new installer, they upgrade, the GUID does not change, `previousVersion` is the old version, and the user's disabled state is kept; a `plugins.manage`-driven plugin verified in turn `validateArchive`, install from a repository source, `conflict` from another source, `replace` replacement (new GUID), switching back to the original source (reusing the original GUID and data), `uninstall`/`install` of a built-in plugin both `unsupported`, it can be disabled, a package with a `../` entry is rejected, a package with `minPlayerVersion: 9.0.0` gets `versionMismatch`, and uninstalling with data kept makes it dormant; when a development load and an installed plugin share an id, only the development load runs.

### Found but not fixed

- The install store's operation receipt limit of 512 (`MAX_OPERATIONS`) is never cleaned up; every seeding upgrade also writes a receipt (the same package reuses the same receipt id). After long use, installs will fail with "operation history full"; receipt eviction needs a separate design.
- `npm run portability` writes the Windows check output inside the repository at `src/host/target/portability` (ignored by `.gitignore`), which is inconsistent with the convention "do not create `target/` in the repository".

### GUI retest fixes (2026-10-21)

The coordinator's real-device test of the Markdown example found 5 problems, now fixed (host `ui_markdown.rs`, `ui/vestra/markdown.slint`, example `sample.md`; SDK branch `store-1.1.1` commit `e42a866`, version 1.1.1, not tagged):

| Problem | Cause | Fix |
| --- | --- | --- |
| Italic and strikethrough not visible | The forced Chinese font (Hiragino Sans GB / Microsoft YaHei) has no italic, and the Slint 1.16 renderer ignores synthesized italics; the strikethrough thickness comes from the font and is under 1 pixel at body size. Latin text is affected too (Latin text also uses this font) | Keep the `Emphasis`/`Strikethrough` styles and additionally color them with `<font color>`: italic amber `#e8a33d`, strikethrough gray `#8a8496` (the format string can only contain literal colors; both are readable in both color schemes) |
| Inline code containing Chinese is not shown | Slint's `Code` style switches the font family to a single monospace font with no fallback | Code text is split by character: ASCII runs use `Code`, other runs use the body font; code blocks become `styled-text` split the same way (hard line breaks per line, empty lines and indentation kept), with ASCII shown in monospace |
| Unfinished tasks show as a missing-glyph box | ☐ is not in the font | `MarkdownBlock.task` (0/1/2); the box and check mark are drawn with Slint shapes; `marker` is empty for task items |
| Header and body alignment differ | Each row allocated column widths from its own content | `MarkdownBlock.aligns` is taken from the GFM delimiter row; every row has equal column widths, header and body are aligned by column; short rows are padded with empty cells |
| The example's remote image returns 400 | Wikimedia no longer accepts non-standard thumbnail sizes | Use `https://github.com/vestrasys.png?size=128` instead (this organization's avatar, 302 to avatars.githubusercontent.com, which also covers the redirect path; verified that the SDK downloader fetches a PNG). The player and plugin repositories are private, so raw.githubusercontent.com cannot serve them; the public SDK repository has no PNG. The SDK downloader now accepts an injected `fetch`, with new tests for HTTP errors, redirects to `http:`, the redirect limit, the size limit and `User-Agent` |

The new fields of the `MarkdownBlock` struct are internal widget-library data (plugins only pass along the value of `plugin.ui.markdown()`) and do not affect plugins. Host tests 288/288 (1 new regression test; the rendering test adds a pixel check for task boxes); SDK 112/112.
