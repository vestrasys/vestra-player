# Plugin Store (`vestra.store`): design and API gaps

> Historical design record (S5 plugin store round, 2026-10-22). Current behaviour: the store plugin's [README](../../src/plugins/store/README.md) and [CHANGELOG](../../src/plugins/store/CHANGELOG.md). Not maintained.

> **2026-10-26 (user rulings, [history §2](../process/rounds/history.md#plugin-platform); store 0.1.6):** the player has no rollback, and the store neither rolls back nor installs an older version: Roll Back / Restore is gone, the confirmations no longer say you can roll back or restore, and the Versions tab offers no version older than the installed one from the same source. To go back, the user uninstalls the plugin and installs a `.vxp`. Plugin management has exactly three operations: install, uninstall, disable/enable. This design has been updated accordingly; the decisions in §9.3, §11 and §12 that allowed rollback or older versions are kept as the record and marked superseded. ("Rollback detection" of a stale repository index, §3.4, is a different thing and stays.)

Date: 2026-10-22. Author: the store plugin author (ecosystem plan S5, step 2.1). Status: **adopted** (coordinator decisions of 2026-10-22, step 2.2, in §11), **implemented** (step 2.4, `src/plugins/store/` 0.1.0). Where the decisions or the implementation differ from the first draft, the text has been changed, and the differences are summarized in §12.

Sources: [ecosystem plan](../product/ecosystem-plan.md) 1.1–1.6 (in particular 1.3 index format, 1.5a ids and install instances, the 12 decisions in 1.6, the record of the second half of S3); [S4 proposal](../architecture/store-api-proposal.md); store-changes (S4 change notice, deleted; the parts still valid are on the [SDK documentation site](https://vestrasys.github.io/vestra-player-sdk/)); [API reference](https://vestrasys.github.io/vestra-player-sdk/modules/_vestra-player_sdk.html); `types.d.ts` of SDK `@vestra-player/sdk` 1.1.2; the plugin repository's `scripts/lib/{signing,semver,publishing,revoked}.mjs` and `keys/index-signing.pub.pem`; the live `https://vestrasys.github.io/vestra-player-plugins/index.json` and `.sig` (fetched 2026-10-22; the signature verifies with the built-in public key, and fails after changing one byte).

## 0. Conclusion

- The store is a **separate built-in plugin** `vestra.store`, in `src/plugins/store/`, released with the player (1.6 item 6). It is responsible for "discover → install → update → uninstall" and, since the Plugin Manager was removed on 2026-10-26, also for disabling, enabling and reloading: it is the only plugin management interface (§1.3). It works on the install library only through the public `plugin.packages`.
- The main interface is a tool window: a list on the left, details on the right (three tabs Description / Changelog / Versions, with a right column for information and permissions); there is also an "Options ▸ Plugins ▸ Plugin Store" settings page that manages third-party repositories and automatic checks.
- The official index is verified with a built-in Ed25519 public key (Node's built-in `crypto`), packages are verified with `sha256` and `size`, and installation goes through `packages.validateArchive` → `install({ source, replace })`.
- **v1 needs no new host or SDK API.** Evidence in §9.1. Two optional improvements (list icons, drag and drop onto tool windows) and one contract clarification (installing a lower version from the same source) are listed as well; none of them blocks implementation.

---

## 1. Positioning and identity

### 1.1 Plugin identity

| Item | Value |
| --- | --- |
| Directory | `src/plugins/store/` |
| id | `vestra.store` (same rule as `vestra.workshop` and the other built-ins; the built-in identity comes from the `bundled` source. The `vestra.` prefix is reserved for built-in plugins; since player 0.4.1 the host refuses to install or test-load other packages whose id starts with `vestra.`, see §12) |
| `minPlayerVersion` | `0.5.1` since store 0.2.1 (images nested in the rows' values, `packages.reload`); `0.2.0` at first (`validateArchive`, `install(options)`, `ui.markdown`) |
| capabilities | `commands`, `ui.window`, `ui.dialog`, `ui.osd`, `ui.drop`, `ui.settings`, `plugins.manage` |
| Display fields | `name: "Plugin Store"`, `author: "Vestra Player"`, `punchline: "Browse, install and update plugins"`, `tags: ["plugins"]`, `license` the same as the main repository |

No `events` and no `playback.*`: the store has nothing to do with playback state. `packages.list` / `onChange` need no capability.

### 1.2 Entry points

| Function | Main menu path | Other entry points | Behavior | Enabled when |
| --- | --- | --- | --- | --- |
| Open the store | `Plugin Store…` (top level, `order: 822`, `group: "windows"`, right after 820 "Plugin Workshop…") | When plugins can be updated, the title changes to `Plugin Store (2 updates)…` | Opens the window; refreshes the index if it is older than 10 minutes | Always |
| Install a plugin package file | — | Drop a `.vxp` on the video area; "Install from File…" in the store window | Verify → confirmation dialog → install | Always |
| Install from a link | — | "Install from Link…" in the store window | Input row → download → verify → confirm → install | Always |
| Repositories and automatic checks | — | "Options ▸ Plugins ▸ Plugin Store" settings page; the "Repositories…" button in the store window opens this page | Settings page | Always |

- Top level rather than under "Settings ▸": like "Plugin Workshop…", it is an entry that opens a work window ([menus.md](../product/menus.md) "Interface forms" lists the workshop as a tool window, `windows` group 800–899), not a setting. **Approved by the coordinator** (decision 2); if the user wants it under "Settings ▸", only `MENU_ORDER` and the path need to change.
- No default shortcut (VS Code's ⇧⌘X has no established meaning here); the user can bind one in "Settings ▸ Keyboard Shortcuts…".
- How the title changes: the command lives in a child scope; when the count changes, the scope is released and the command added again, with the same `order` (the approach in reference §4).

### 1.3 The only management interface

**Since 2026-10-26 the store is the only place where plugins are managed** (user ruling, [history §2](../process/rounds/history.md#plugin-platform); store 0.2.0, finished in 0.2.1). The Plugin Manager (`vestra.plugins`) and "Settings ▸ Plugin Manager…" were removed; what it did moved into the store's Installed tab. Plugin management is install, uninstall, disable/enable, nothing else (no rollback, no older version, no Undo).

| Operation | Where in the store | API |
| --- | --- | --- |
| Browse repositories, search, README, permissions, version history | All / Updates tabs, details | index (§3) |
| Install (repository, link, file, drop), replace another source, update (a plugin disabled before an update or reinstall stays disabled and never runs, as in VS Code: after host 0.6.3 the host keeps it disabled and does not activate it, [store API §1.3](store-api-proposal.md#13-rules); up to 0.6.3 the host enabled it on commit, so Store 0.2.9 calls `setEnabled(id, false)` right after, which is harmless on a newer host; a revoked version the store disabled is re-enabled once its fix is committed: Store 0.2.10 calls `setEnabled(id, true)` after the commit, because it must also work on players up to 0.6.3, which ignore `enabled`; `install({ enabled: true })` does it in the install with SDK 1.5.55 or later on player 0.6.4 or later, §3.10. The update itself is committed without an activation check (VS Code semantics, no rollback): a broken version shows as failed when it is enabled) | Row button (Install, Update), details, ⚙ menu | `packages.validateArchive`, `install`, `setEnabled` |
| Uninstall (with or without data) | `…` menu, details | `packages.uninstall` |
| Disable / enable (no confirmation, user ruling: it can be undone at once) | The row's ⋯ Manage menu (a row with an update has only its Update button); details | `packages.setEnabled` |
| Reload (running plugins, built-ins included, and failed ones; not a disabled one) | `…` menu; details for a failed plugin (last button) | `packages.reload` (player 0.5.1) |
| Open the data folder; status, package, data folder, data size, failure reason | `…` menu (last entry), details | `PluginInfo`, `system.openPath` |
| What a running plugin was refused (a panel position, Previous/Next, a file type held by another plugin) | Warning tag on the row, notices in the details | `PluginInfo.warnings` |
| Development loads (`--plugin`) | Installed tab with a Dev tag; disable/enable for this run only, never uninstalled | `PluginInfo.origin` |
| Revocation handling (automatically disable revoked versions) | Notice, details | §3.10 |

Built-ins are listed first under Installed. They can be neither uninstalled nor disabled; their `…` menu offers Reload and Open Data Folder. The store cannot disable or reload itself (the player refuses `invalid`), so a user who wants no network access turns off the automatic check on the settings page: the store then goes online only when its window opens or Refresh is clicked.

The host's install library is the single source of truth: the store follows `packages.onChange`, and reads `packages.list` again after a refusal (`missing`, a refused uninstall), when nothing changed for `onChange` to report. Another plugin with `plugins.manage` may still change the library; the host answers concurrent operations with `busy`.

---

## 2. Interface

### 2.1 Main window (tool window `ui/store.slint`, default 900×600, minimum 760×480)

```text
┌ Plugin Store ────────────────────────────────────────────────────────────────────────┐
│ [🔍 Search name, author, tag…   ]  (All)(Installed)(Updates 2)  [Tag: All ▾] [Sort: Name ▾] │
│                                     [Install from File…][Install from Link…][Repositories…][Refresh] │
├───────────────────────────────┬──────────────────────────────────────────────────────┤
│ YouTube                0.2.0  │ ┌────┐ YouTube                                       │
│ zysilm · Official   Installed │ │icon│ zysilm · Official repository                  │
│ Paste a link to play or dow…  │ └────┘ Paste a link to play or download a single     │
│                               │        public YouTube video                          │
│───────────────────────────────│        [Update to 0.2.1] [Disable] [Uninstall…] │
│ Bilibili Video         0.2.0  │ ⚠ Hint bar (revoked / unofficial / incompatible / source conflict, see 2.3) │
│ zysilm · Official             │ ┌──────────────────────────────────┬────────────────┐ │
│ Paste a link to play a sing…  │ │ (Description) (Changelog) (Versions) │ Version 0.2.0 │ │
│───────────────────────────────│ │                                  │ Installed 0.2.0│ │
│ Cool Visualizer        1.3.0  │ │  MarkdownView                    │ Updated 2026-10-21│ │
│ someone · Unofficial: Foo repo│ │  (README / CHANGELOG)            │ Repo  Official │ │
│ …                             │ │                                  │ License Apache-2.0│ │
│                               │ │                                  │ Code  Open ↗   │ │
│                               │ │                                  │ ─ Permissions (7) ─ │ │
│                               │ │                                  │ · Control playback │ │
│                               │ │                                  │ · Open tool windows │ │
│                               │ │                                  │ · …            │ │
│                               │ └──────────────────────────────────┴────────────────┘ │
├───────────────────────────────┴──────────────────────────────────────────────────────┤
│ Official repository updated 10:21 · Foo repo can't connect (showing cache from yesterday 18:02)  [Downloading 42% Cancel] │
└──────────────────────────────────────────────────────────────────────────────────────┘
```

- **List** (width 300, `ListView`, row height 52: name + version; author · repository mark + state; one-line summary, truncated with an ellipsis, full text on hover). One row corresponds to one "(repository, id)"; if the same id appears in two repositories, that is two rows, each marked with its repository. Under All (store 0.2.11, user ruling 2026-11-03) an installed id has one row: the installed row (its own repository's, a sideloaded or workshop one, a built-in's) stands for every repository entry of that id, keeps its state and its control, says "Also in <repository>" in its details and offers "Replace with <version> from <repository>…" (the replace question of §2.3) last in its ⋯ Manage menu, or, when the row shows Update instead, last in its details; asking for such an entry (`select`) shows the installed row. A plugin that is not installed keeps one row per (repository, id). Installed and Updates are unchanged. Installed plugins that are not in any repository (built-in, link, file, workshop, from a repository that was removed) are listed too, with the source as the repository mark ("Built-in", "Local file", "Link", "Plugin Workshop").
- **Row logos (store 0.2.1, player 0.5.1, which takes images nested in values; G1 closed):** a 32px logo on each row, decoded at 64 (`maxSide`), so even a thousand rows stay far under the player's 32 MB of decoded pixels per view. An installed plugin shows the icon from its own package (copied into `<data>/icons/`, so offline too); a repository row shows the index's `iconUrl` through `ui.remoteImage`, fetched only once a row is shown and, for a third-party repository, only from the index's own host; otherwise the store's own icon. The details icon is decoded at 112.
- **Filters** (tabs in this order; store 0.2.11 replaced Available with All, user ruling 2026-11-03; the last tab is kept in settings.json, the first open is All): "All" = the plugins of all repositories, installed or not, + installed plugins no repository lists (each row with its state and buttons; without any index only the latter, with a calm note under the list); "Installed" = those in `packages.list()` with `origin: "installed"`; "Updates" = those with an updatable version (§3.6). Search is case-insensitive and matches name, id, author, summary and tags (substring; Chinese by character). The tag drop-down lists all tags of the currently visible plugins. Sort: name (default, by `localeCompare("zh")`) or update time (`published` of the latest version, newest first). Download counts are not shown (1.6 item 9).
- **Details**: icon (`iconUrl` downloaded and cached, then `ui.image`; when there is no icon or it fails, the Vestra Player logo bundled in the package, `assets/logo.png`), name, author, repository, summary, action buttons, hint bar; three tabs on the left, information and permissions in the right column.
  - "Description": README (`readmeUrl`, or the README in an installed plugin's package); "Changelog": `changelogUrl` or the CHANGELOG in the package; when missing, "The author provided no description" / "The author provided no changelog".
  - "Versions": the version history table (version, release date, `minPlayerVersion`, state: current / incompatible / revoked); each installable old version has an "Install" button at the end of its row.
  - Right column "Permissions": the manifest's capabilities translated into plain language per §2.4, high-risk items in amber; at the end, a fixed grey line "A plugin is a complete program. These permissions only limit how it uses the player's features; they do not limit reading and writing files or network access." (1.6 item 4).
- With nothing selected the right side shows "Select a plugin on the left to see its details"; when the list is empty it shows the reason (offline with no cache, no filter results).
- Bottom status line: the refresh result of each repository, the progress of ongoing downloads / installs and "Cancel".
- "Install from Link…" expands a `UrlInputRow` above the list (already in the widget library, `confirm-text: "Install"`); Esc collapses it.
- Open / close: closing the window (`onClose`) only stops README loading in progress; downloads, installs and background checks are not affected, and the next open returns to the previously selected plugin. On open, focus goes to the search box.

### 2.2 Markdown rules

| Item | Rule |
| --- | --- |
| Source | Repository plugins: GET `readmeUrl` / `changelogUrl` (≤ 256 KB, 10 seconds, same rules as the §3.2 downloads); installed and not in a repository: read the manifest's `readme` / `changelog` in `info.directory` (default `README.md` / `CHANGELOG.md`) |
| `base` | Repository plugins use `readmeUrl` itself (relative images and links resolve against it); local READMEs use the package directory |
| Remote images | **On for the official repository** (`remoteImages: true`); **off for third-party repositories and links**. When the README contains `![…](http…)`, the details page hint bar shows "N remote images blocked [Show Images]"; clicking it re-renders with `remoteImages: true` for this plugin only, for the current run. Reason: remote images send the user's IP to arbitrary third-party servers (tracking pixels) |
| Relative images in local READMEs | They are in another plugin's package directory, and `ui.markdown` only reads this plugin's package and data directory, so the alternative text is shown. Accepted (README images of sideloaded plugins are rare; cross-plugin read paths are not opened for this; S4 §10 reached the same conclusion) |
| Links | `link-clicked(url)`: `http(s)` opens with `system.openUrl`; `#anchor` is ignored (MarkdownView has no table-of-contents jumps); other schemes are ignored and the status line shows "This kind of link cannot be opened" |
| Cache | README / CHANGELOG text is cached in memory by URL (for this run); switching plugins does not download again |

### 2.3 States and buttons

**Rows (store 0.2.12, user ruling 2026-11-03, VS Code's extension list):** every row has exactly one control, in the same place at the bottom right: Install for a plugin not installed, Update while a newer version exists, otherwise a ⋯ Manage button (tooltip and accessible name "Manage") whose menu keeps one fixed order: Disable·Enable, Uninstall…, Reload (not while disabled), Open Data Folder, then, under All, "Replace with <version> from <repository>…". Built-ins have Reload and Open Data Folder only; a development load has no Uninstall…. Right before the version a grey ✓ (accessible name "Installed") marks an installed plugin; a disabled one shows "Disabled" there instead and its row is dimmed, a failed one "Failed"; a row with an update or a development load has no ✓. The second line keeps the other states ("Update → x.y.z", Revoked, Built-in, Incompatible…).

**Details:** an installed third-party plugin's buttons keep one fixed order, so disabling or enabling it changes words, never places (store 0.2.9, VS Code's extension page): [Update to x.y.z] (while a newer version exists) [Disable | Enable] [Uninstall…], then Reload, for a failed plugin, or for a running one whose row shows Update (its row has no ⋯ menu); for such a row, the details also end with "Replace with <version> from <repository>…" when another repository has the plugin (under All).

The state is derived by comparing "this row (repository R, id)" with the install library entry of the same id (`packages.list()`); `v*` is the "latest installable version" chosen in §3.5.

| State | Condition | Main button | Hint bar |
| --- | --- | --- | --- |
| Not installed | The install library has no such id | "Install" (installs `v*`) | Third-party repository: amber "Unofficial repository: not reviewed by Vestra Player" |
| Installed (latest) | Same id, source = R, version ≥ `v*` | "Disable" ("Enable" while disabled), "Uninstall…" | — |
| Update available | Same id, source = R, `v*` higher | "Update to x.y.z" (also while disabled; the plugin stays disabled), "Disable" or "Enable", "Uninstall…" | When the new version adds permissions: "The new version needs new permissions: …" |
| Installed from another source | Same id, source ≠ R, not built-in | "Replace…" | "Installed <version> from <source>. Replacing it uses new, empty data; the original data is kept" |
| Same id as a built-in plugin | The id is `builtin: true` in the install library | None (button disabled, hover "Built-in plugins are updated with the player") | "Same id as a built-in plugin of the player; cannot be installed from here" |
| Incompatible | No version satisfies `apiVersion` and `minPlayerVersion` | None | "Needs Vestra Player ≥ x.y.z (current a.b.c)" or "Needs a newer plugin API (vestra/2)" |
| Revoked (installed version) | The installed version falls in R's `revoked` range | "Update to x.y.z" when a fixed version exists (updating a version the store disabled enables the fix: Store 0.2.10 calls `setEnabled(id, true)` after the commit, since the host keeps a disabled plugin disabled across an update); "Enable Anyway…" while the plugin is disabled (confirm the risk first, §3.10), "Disable" otherwise; "Uninstall…" | Red "This version has been revoked: <reason>. The store has disabled it." (after the risk is confirmed and it is enabled again: "…You confirmed the risk and enabled it again; the store will not disable it again.") |
| All versions revoked | Not installed and all compatible versions are revoked | None | "This plugin has been revoked: <reason>" |
| In progress | A download / install / uninstall is running for this row | "Cancel" | Progress |
| Only on this machine (not in any repository) | In the install library, but no repository has this id or the source repository was removed | "Disable" or "Enable", "Uninstall…" | When the source is a link / file / workshop: "Sideloaded plugin: not reviewed; the store does not check it for updates" |

Development loads (`origin: "development"`) and test loads are not in "Installed" and do not affect the table above (they take precedence for this run; the store shows the install library's state as usual and writes "This run uses the development-loaded version" in the hint bar).

### 2.4 Permissions in plain language

The store has its own table (plugins do not share code). Unknown capability names (only in newer players) are shown as "Unknown permission: <name>".

The English wording of each capability and whether it is highlighted (⚠) follow the SDK's `CAPABILITY_INFO` (`label`, `risky`); the table is in the "Shown in the store as" and "Highlighted" columns of [Reference: Capabilities](https://vestrasys.github.io/vestra-player-sdk/documents/Capabilities.html) on the SDK documentation site. The store's `src/plugins/store/labels.ts` matches it entry by entry (checked by `src/tests/sdk/host-agreement.test.ts`); the translations are in the store's `locales/`.

### 2.5 Confirmation dialogs (`dialogs.confirm`)

Every install is confirmed first (1.6 item 5 "manual confirmation by the user"; permissions are listed before downloading, 1.6 items 4 and 10):

| Scenario | message | detail | confirm |
| --- | --- | --- | --- |
| Install from the official repository | `Install “YouTube” 0.2.0?` | `From the Vestra Player official repository, author zysilm.\n\nIt will be able to:\n· Control playback…\n\nA plugin is a complete program. These permissions only limit how it uses the player's features; they do not limit reading and writing files or network access.` | Install |
| Install from a third-party repository | `Install “X” 1.3.0 from an unofficial repository?` | `Repository: Foo (https://foo.example/). This repository is not reviewed by Vestra Player; its plugins may read your files or use the network. Only install plugins from authors you trust.\n\nIt will be able to: …` | Install Anyway (`destructive`) |
| Link / file (sideload) | `Install the plugin package “X” 1.0.0?` | `From <link or file name>; not reviewed by any repository and not updated automatically. … It will be able to: …` | Install Anyway (`destructive`) |
| Update (same permissions) | `Update “X” to 0.2.1?` | `Current: 0.2.0.` | Update |
| Update (more permissions) | `Update “X” to 0.3.0? The new version needs more permissions` | `New: · Read and write the clipboard …` | Update |
| Replace another source | `Replace the installed “X”?` | `The current X 0.2.0 comes from <source>. After replacing, the new plugin uses new, empty data; the original plugin's data is kept and is used again when you reinstall it from the original source.` | Replace (`destructive`) |
| Uninstall | `Uninstall “X”?` | `Its menus, windows and processing effects are removed immediately.` + check box "Also delete its settings and data" (unchecked by default) | Uninstall (`destructive`) |

There is no confirmation for an older version from the same source: the Versions tab does not offer one (2026-10-26, Q2).

When `detail` exceeds 2000 characters, the permission list is cut to the first 20 items plus "N in total".

---

## 3. Data flow

### 3.1 Repositories and addresses

- **Official repository**: `https://vestrasys.github.io/vestra-player-plugins/index.json`, written in the code; it cannot be removed or turned off on the settings page (the official repository is the reason the store exists; users who do not want network access turn off the automatic check, and the store then goes online only when its window opens or Refresh is clicked; the store is a built-in and cannot be disabled).
- Addresses entered by the user are normalized per 1.1: WHATWG parsing; only `https:` is accepted (plus `http://localhost`, `http://127.0.0.1` and `http://[::1]`, so authors can test their own repositories, with an extra warning); `#…` is removed; a path ending in `.json` is used as is, otherwise `/` is appended and then `index.json`. The normalized `href` is the `PackageSource.url` (the host applies the same normalization again and compares character by character, `package_manager.rs` `normalize`), so "updates only from the original source" is decided on this string.
- The address of `index.json.sig` = the index address + `.sig`.

### 3.2 HTTP (Node `fetch` in the plugin process)

The plugin process is a complete Node (the player bundles Node 26; the start of the reference: "plugins can use the Node standard library, the file system, the network and npm dependencies"), so no host API is needed. All requests go through one function `get(url, limits)`:

| Rule | Value |
| --- | --- |
| Scheme | Every hop must be `https:` (local exception as in 3.1); a redirect to `http:` or another scheme fails |
| Redirects | `redirect: "manual"`, followed manually ≤ 5 times (GitHub Releases downloads redirect 302 to `release-assets.githubusercontent.com`, tested) |
| Timeouts | Connect + first byte 15 seconds; whole request: index 30 seconds, packages `max(60 seconds, size / 100 KB/s)` |
| Size | Read as a stream and count the **decoded** bytes; abort as soon as the limit is exceeded; a `Content-Length` above the limit is refused outright. When the response has a `Content-Encoding` (GitHub Pages uses gzip for READMEs), `Content-Length` is the compressed byte count while `fetch` delivers decompressed content; in that case `Content-Length` is not used, and only the decoded count is used for the limit, the exact size and the sha256 check (on-device acceptance S1). `Accept-Encoding: identity` was not chosen: servers and CDNs may ignore it, while counting decoded bytes is correct in both cases and still stops compression bombs |
| Limits | `index.json` 4 MB; `.sig` 1 KB; README / CHANGELOG 256 KB; icon 512 KB; repository packages the index's `size` (must be equal); link packages 64 MB (same as the host's `.vxp` limit) |
| Headers | `User-Agent: VestraPlayer/<app.version> vestra.store/<version>`; no cookies (Node fetch sends none anyway); conditional requests with `If-None-Match` / `If-Modified-Since` |
| Cancel | One `AbortController` per request; the window's "Cancel" and scope release (plugin disabled) both abort |
| Concurrency | ≤ 4 repositories at once when refreshing; only one package downloaded and installed at a time (the host also accepts only one operation per id) |

### 3.3 Signature verification

```ts
import { createPublicKey, verify } from "node:crypto";
// keys/index-signing.pub.pem (plugin repository), base64 of the SPKI DER; tests check its fingerprint.
const OFFICIAL_KEYS = ["MCowBQYDK2VwAyEA6DswGEDcZFAbqpeRjvANiIAI7aziHk6sYLnHRpW0pdY="].map((b64) =>
  createPublicKey({ key: Buffer.from(b64, "base64"), format: "der", type: "spki" }));

export function verifyIndex(bytes: Buffer, sigText: string, keys = OFFICIAL_KEYS): boolean {
  // One or more signatures separated by whitespace (room for key rotation, §6.3); each is 64 bytes in base64.
  return sigText.trim().split(/\s+/).some((part) => {
    if (!/^[A-Za-z0-9+/]+={0,2}$/.test(part)) return false;
    const raw = Buffer.from(part, "base64");
    return raw.length === 64 && keys.some((key) => verify(null, bytes, key, raw));
  });
}
```

- The signature is verified over **the raw bytes received**, and `JSON.parse` only runs after it passes; never re-serialize, never convert line endings.
- Official repository: no `.sig` (404), `.sig` read failure, or signature failure → this refresh fails, **the last verified cache stays in use**, and the status line shows the error (§5).
- Third-party repositories: v1 does not verify signatures (we have no public keys for them) and does not read `.sig` even when present; the interface always marks them "Unofficial" (§4). A third-party repository whose index claims `official: true` **has that field ignored**. "Official" has only one criterion: the address is the built-in official address, and the signature verifies with the built-in public key.
- Tests inject test keys (constructor parameter, §8); product code has no path to replace the public key from settings or environment variables.

### 3.4 Parsing and validating the index

Ported from `indexProblems` in the plugin repository's `publishing.mjs`, but **lenient about unknown keys** (ignored), leaving room for `extensions` and later fields; `schema` must be `1`, otherwise the whole index is refused ("The repository format is newer than this player; please update Vestra Player").

| Level | When invalid |
| --- | --- |
| Top level (`schema`, `name`, `plugins` array, `revoked` array) | Whole index refused; the cache is used |
| A single plugin (id rules as in reference §2; `versions` not empty; field types; URLs must be `https:`) | Only this entry is dropped, one log line |
| Store assets (`iconSha256`/`iconSize`, `readmeSha256`/`readmeSize`, `changelogSha256`/`changelogSize`, next to the corresponding URLs) | Both null or missing: not checked (old index). Wrong format (not 64 lowercase hex characters / not a non-negative safe integer, or only one of the two given): this asset is unavailable (URL set to empty), the plugin is kept. Given and valid: download limit = min(original limit, Size); after downloading, the size must be equal and the sha256 must match, otherwise the asset is discarded — not shown, not cached; README / changelog show a load failure ("Couldn't load the description: checksum does not match the description" and so on), and the icon falls back to the default icon. The icon cache is named by `iconSha256`, and the size and sha256 are checked again before each use from the cache; on mismatch it is deleted and downloaded again. Third-party repositories are handled the same way (as an integrity check only: their index has no signature, so the hash cannot stop the repository author, only transport corruption and a replaced CDN) |
| A single version (`version` strictly `x.y.z`; `sha256` 64 lowercase hex characters; `size` a positive integer ≤ 64 MB; `downloadUrl` `https:`; `capabilities` an array of strings; `published` parseable) | Only this version is dropped |
| `revoked` entries (id; `versions` parseable as semver syntax; `reason` 1–500 characters) | Only this entry is dropped (one log line; the official index is validated in CI, so this should not happen) |
| Counts | Plugins ≤ 2000, versions per plugin ≤ 200, the excess is ignored; more than 1000 `revoked` entries **refuses the whole index** (truncation would silently miss revocations; on failure, better not use it at all) |
| Display field lengths | Truncated for display per the manifest rules (name 64, summary 120, description 2000, tags 8×24), shown only as plain text |

Duplicate ids: the first is kept. Versions keep the original `versions` order (new to old), and the client sorts by version number once more, without relying on server order.

**Monotonic sequence number `generated`** (since plugin repository 68ff01b; a positive integer at the top level, +1 on every publish): checked only after the official index has been verified and parsed. Missing or not a positive integer counts as 0. `meta.json` records the largest accepted value (it only grows): a new index with a smaller value is judged a replay (refused, cache kept, status line as below); equal (the same index or a 304) and larger are accepted, and the maximum is updated. So once a value ≥ 1 has been seen, an index without `generated` is always refused — only old indexes from before the field existed lack it. At startup, reading the cache also requires the cached index's value to be ≥ the recorded maximum. The `seen` check below stays in place; the two complement each other.

**Rollback (freeze) protection** (revised after the 2026-10-22 security review): for the official repository, the cached `meta.json` records `seen`: **all** versions (per plugin) and **all** revocation entries (id + range) that have appeared in any verified index. It only grows: when a plugin disappears from the index, the versions it was seen with stay in `seen`. A new index is judged a replay of an old index in these cases, and this result is refused, the cache stays in use, the status line shows amber "The official repository returned an old index; this update was ignored", and one log line is written: ① a listed plugin lacks a version that was seen (including one that was delisted and reappears with old versions missing); ② a revocation entry that was seen is missing. A plugin that is entirely absent from the index is still accepted (normal delisting). This relies on a plugin repository convention: published version entries are kept as they are, and `revoked.json` is append-only (there is no way to narrow a mistaken revocation with a new entry; the only way out is a new version; the plugin repository is to write this into its rules). With `generated`, any signed index older than the accepted one is refused, including the kind where "one plugin happens to be absent" (it could only be accepted before the field existed, while the store had not yet seen any sequence number).

### 3.5 Version selection

```text
compatible(v) = v.apiVersion === "vestra/1"
             && (v.minPlayerVersion === null || compare(app.version, v.minPlayerVersion) >= 0)
revoked(id, v) = R.revoked contains an entry with the same id and satisfies(v.version, versions)
v* = the first version, from highest to lowest version number, that is compatible and not revoked
```

- `app.version` comes from `plugin.app.version` (the host's Cargo version, 0.2.0); comparison uses only the three numeric parts (consistent with the host's rule, pre-release suffixes ignored).
- `apiVersion` is hard-coded to `"vestra/1"`: the store is itself a `vestra/1` plugin, so a player that can run it supports `vestra/1`. When a future player supports two major versions at once, a list of supported versions in `plugin.app` will be requested per §4; not needed now.
- `satisfies` / `compareVersions` / `parseRange` are ported unchanged from the plugin repository's `scripts/lib/semver.mjs` as `semver.ts` (same syntax: exact versions, `<`, `<=`, `>`, `>=`, `=`, space-joined, `||`, `*`; no `^` or `~`). Tests use the same set of cases.
- "Install Other Version" (the Versions tab) lists all versions; incompatible and revoked rows are disabled with the reason, and a version older than the installed one from the same source has no Install button (2026-10-26, Q2).

### 3.6 Refresh and update checks

| When | Action |
| --- | --- |
| 30 seconds after the plugin is activated (does not slow down player startup) | If "Check for updates automatically" is on (default on): refresh all repositories and compute the number of updates (decision 6: check once on every startup, regardless of when the last check was) |
| While running | Look once an hour; if ≥ 24 hours have passed since the last full check (automatic or "Refresh"), refresh again; compared with wall-clock time, so a check is made up after the computer wakes from sleep |
| Opening the store window | Refresh any repository whose cache is older than 10 minutes (Pages' `max-age=600`); show the cache first |
| "Refresh" button | Refresh everything immediately, regardless of time |
| Adding a repository | Read it immediately (if that fails it is not added, §4) |

- **Update available** = in the install library, `source.kind === "repository"`, `source.url` equals the address of some repository, and that repository's `v*` for this id is higher than the installed version. A higher version of the same id in another repository **does not count** (1.5a "updates only from the original source"). Built-in plugins (source `bundled`) never count (1.6 item 6).
- When updates are available: the menu title carries the count; the first time in this run, `osd.show("Plugin Store: plugin updates available (2)")` (failure ignored). **No automatic install** (1.6 item 5).
- With automatic checks off, the network is only used when opening the window and clicking "Refresh" (revocation checks then also happen only at those times; the settings page says so).

### 3.7 Cache and offline

```text
<dataDirectory>/
  settings.json                  plugin.storage.json("settings"): repository list, automatic checks, last check time, handled revocations
  repos/<first 16 hex digits of sha256(url)>/
    index.json  index.json.sig   raw bytes (for the official repository, written only after the signature verifies)
    meta.json                    { url, etag, lastModified, fetchedAt, versionsSeen }
  icons/<sha256(iconUrl)>.png|jpg   ≤ 512 KB, PNG/JPEG decided by content; evicted by access time when the total > 16 MB
  downloads/<random>.vxp(.part)     deleted after install completes or fails; leftovers deleted at startup
```

- All writes are "temporary file + rename"; on Windows, when the rename target exists, it is deleted first (file names are random each time, so normally there is no clash).
- At startup the cache is read, and the official cache is **verified again** before use (in case the data directory was modified).
- A refresh that gets `304` only updates `fetchedAt`. On network failure the cache is shown with its time: "Can't connect to <repository name>, showing the cache from <time>". With no cache and no connection: that repository's plugins do not appear in the list, and a hint is shown at the top of the list.
- The state of installed plugins (installed, source, built-in) always comes from `packages.list()` and is not cached.

### 3.8 Install: repository source

```text
1. The user clicks "Install / Update / Install This Version" → state check (2.3) → confirmation dialog (2.5, permissions taken from the index entry)
2. Download downloadUrl → downloads/<random>.vxp.part, computing sha256 and counting bytes while downloading
3. Byte count ≠ size or sha256 ≠ entry → delete, fail with "The downloaded file does not match the index"
4. Rename to .vxp → candidate = await packages.validateArchive(path)
   (an id starting with `vestra.` already fails here with `reserved`; the host's original text is shown, §5.2)
5. Check candidate.manifest: id and version equal the entry, capabilities equal the entry as a set → otherwise fail with "The plugin package does not match the repository's description"
   (the user agreed to the permissions listed on the entry; the package must not carry extra permissions)
6. change = await candidate.install({ source: { kind: "repository", url: R.url } })
   - conflict → replace confirmation dialog → candidate.install({ source, replace: true })
   - unsupported → "Same id as a built-in plugin of the player; cannot be installed"
   - versionMismatch → "Needs Vestra Player ≥ x.y.z" (only reached when the index has a wrong minPlayerVersion)
   - busy → "Plugins are being installed or cleaned up; try again later"
7. Follow change.onChange until done: committed → osd "Installed X 0.2.0"; failedDegraded → "Installation failed; the previous version was restored";
   failed → "Installation failed: <message>"; cancelled → "Cancelled"
8. finally delete the file in downloads
```

- **Update** = install a higher version from the same source (the host treats the same source as an upgrade and keeps the instance and data; it keeps no previous version).
- **No rollback and no older version** (2026-10-26, Q2/Q3): the store has no Roll Back / Restore action and does not install a version older than the installed one from the same source (the Versions tab offers none; `installFromRepository` also returns without asking if one is requested). To go back, the user uninstalls the plugin and installs a `.vxp` (from a file or a link).
- **Cancel**: during download, abort the `fetch`; during install, call `change.cancel()` (the host may answer `tooLate`; the result is shown accordingly).
- **Replace and dormant instances**: when replacing, the store does not need to know GUIDs or dormant instances: if the new source happens to have a dormant instance with the same id, the host automatically reconnects its data (S4 §1.3 "switching back"). The confirmation dialog text says "the original plugin's data is kept and is used again when you reinstall it from the original source".

### 3.9 Install: link, file, drop (sideload)

| Source | Flow | `PackageSource` |
| --- | --- | --- |
| Link | `UrlInputRow` input → normalize (`https:` only) → download (≤ 64 MB, no expected sha256) → same as 3.8 from step 4, but step 5 only checks "it parses", and permissions are taken from `candidate.manifest` (the permissions are only known **after** downloading; the confirmation dialog comes after validation and before install) | `{ kind: "link", url }` |
| File | "Install from File…" `dialogs.openFile({ title: "Choose a plugin package (.vxp)" })` | `{ kind: "file" }` |
| Drop | `drops.provide({ extensions: ["vxp"], drop })`: a `.vxp` dropped on the **video area**; several dropped at once are handled one by one (one confirmation dialog each) | `{ kind: "file" }` |

- File and drop do not copy the file: `validateArchive(original path)` directly; the host unpacks it to a temporary directory itself.
- The file dialog cannot restrict extensions (`openFile` only has `title`); if another file is chosen, `validateArchive` returns `invalid` and the hint is "This is not a valid plugin package". No request (see the G4 note).
- When dropping while the store window is not open: open the window first and select this package's details (shown from the manifest), then show the confirmation dialog.
- For the host, link and file are the same source ("manual install"): installing a package with the same id from a new link is an upgrade, not a conflict (confirmed by the S4 coordinator).
- For sideloaded plugins the store **does not check for updates and does not handle revocations** (there is no way to know which repository entry they correspond to).
- Dropping on the store window itself has no effect (the host ignores drag and drop on tool windows, reference §6.7). The window shows a grey line "You can also drop a .vxp file on the player's video area to install it". See gap G2 (optional).

### 3.10 Revocation

1. After every successful repository refresh, for each installed plugin: if `source.kind === "repository"` and `source.url` = R.url, and the installed version `satisfies` some `revoked` entry of R → revocation hit.
2. On a hit that **has not been handled** for this (repository, id, version, range): `packages.setEnabled(id, false)` (not called again if already disabled), record it as `"disabled"` in `revocations` in `settings.json`, then show a confirmation dialog: message `Disabled the revoked plugin “X” 0.2.0` (with several, "Disabled N revoked plugins", merged into one dialog); detail lists the reason for each, "You can update to 0.2.3 in the Plugin Store." when a fixed version exists, and explains that "Enable Anyway…" is available on the details page; confirm "Open Plugin Store" (opens the window and selects it) / cancel "OK".
3. **Enabling again requires confirming the risk first** (decision 4):
   - "Enable Anyway…" on the store's details page: confirmation dialog `Enable the revoked “X” 0.2.0 anyway?` (`destructive`), detail with the revocation reason and "Continuing to use it may be risky. After you confirm, the store will not disable it automatically again, and the details page will keep showing a warning."; when a fixed version exists, it suggests updating instead. After confirmation, record `"accepted"` first, then `setEnabled(id, true)`.
   - It is enabled again outside the store (another plugin with `plugins.manage`; formerly the Plugin Manager): the store sees through `packages.onChange` that a plugin recorded as `"disabled"` is running again, and afterwards shows a confirmation dialog `“X” 0.2.0 has been revoked. Keep using it?`, confirm "Keep Using" (recorded as `"accepted"`) / cancel "Disable" (disabled again; Esc also disables). Such an enable does not go through the store; this is the closest the store can get to "confirm first". If it is enabled while the store has it disabled (or while the store itself is disabled), the store asks the same question the next time it runs.
   - Once recorded as `"accepted"`, the store no longer disables it or asks; the red hint on the details page stays.
4. When the dialog is `busy` (another dialog is open), the notice is shown again after 60 seconds; with no interface (`unavailable`) only a log line is written (the plugin is still disabled). If disabling fails (`busy`, an install is running), it is not recorded as handled, and the next round tries again. The check runs after every repository refresh and after every `packages.onChange` (using the verified cache, without network), so a revoked version that becomes installed in any other way is handled too.
5. For installed plugins, revocation is judged by the source repository's `revoked`, and **does not require the index to still list the plugin**: after a plugin is delisted (or all its entries are dropped as invalid), the same repository's `revoked` still applies (2026-10-22 security review: "delist + revoked `*`" is the most natural way to deal with a malicious plugin). Such a row has no repository entry, the state is "Revoked", and it only has "Enable Anyway…" and "Uninstall…"; a delisted plugin that is not revoked has the hint "The repository no longer offers this plugin; the store no longer checks it for updates (it still checks for revocations)".
6. Revocation only applies to plugins installed from **the same source**: a repository can only revoke what it published itself; a third-party repository therefore cannot disable plugins of the official repository or of other repositories. Built-in plugins are not handled (`bundled`).
7. Revocation entries also affect installation: a revoked version is never chosen as `v*`, is marked "Revoked" in the version history and cannot be installed.

---

## 4. Third-party repositories

1.6 item 4: third-party repositories and sideloading carry clear warnings in the interface. Scope of the v1 release:

"Official / unofficial" in the store refers to the **repository**, not to a category of plugin: the only official plugins are the built-in plugins installed with the player (`vestra.*`), and all plugins in repositories are third-party plugins; plugins in the official repository (such as `zysilm.youtube`) are only hosted in the official repository and reviewed more strictly. The index's `official: true` also only means "this is the official repository".

| Item | v1 |
| --- | --- |
| Add | Settings page "Add Repository…": input row (`UrlInputRow`) → normalize → read immediately → added only if it parses, with a confirmation dialog: `Add the unofficial repository “<name>”?` detail `Address: …\nIts plugins are not reviewed by Vestra Player and may read your files or use the network. The store will show this repository's plugins but will not judge for you whether they can be trusted.`, confirm "Add" |
| Remove | "Remove" on each row of the settings page list; removing only stops reading it, installed plugins are untouched (they become "only on this machine", no longer checked for updates and revocations), and the confirmation dialog explains this |
| Count | At most 16 third-party repositories |
| Display | Every row and the details are marked "Unofficial: <repository name>"; the repository name comes from the index (≤ 64 characters), followed by the host name to prevent impersonation (a name like "Vestra Official (foo.example)" is recognizable at a glance) |
| Signature | Not verified (no public key); relies only on HTTPS and each package's `sha256` (protects against transport corruption and mirror replacement, not against the repository author) |
| Same id | Same id as the official or another repository: shown as two rows; installing the same id from another source goes through the replace confirmation |
| Not done | Per-repository public keys, TOFU, a repository-level "trust" switch, repository priority order |

Why no TOFU: third-party repositories usually do not sign at all; TOFU only protects against "the repository address is hijacked", which HTTPS already does; and once a key changes, the user only sees a warning they cannot understand. If needed later, a "Public key" field can be added to the settings page (a pinned key, not TOFU); the client code is already written for "a set of trusted public keys". §11 decision 5 adopted this approach.

---

## 5. Error handling and user-visible text

Principle: on failure nothing is changed (the host's install is atomic); errors go in the details page hint bar or the status line, plus one `osd.show` (failure ignored); the `message` text is not used for decisions, only the `code` is (reference §13).

### 5.1 Repositories and network

| Situation | Text (status line / list hint) |
| --- | --- |
| Refreshing | Updating <repository name>… |
| Success | <repository name>: updated at <HH:mm> |
| Unchanged (304) | <repository name>: up to date at <HH:mm> |
| Can't connect / DNS / TLS error / timeout | Can't connect to <repository name> (<reason>); showing the cache from <date time> |
| Same, with no cache | Can't connect to <repository name>; its plugins are not available for now |
| HTTP 4xx/5xx | <repository name> returned error <status code> |
| Official signature missing | The official repository has no signature file; this update was ignored |
| Official signature mismatch | The official repository's signature check failed (it may have been tampered with); this update was ignored |
| Rollback detected | The official repository returned an old index; this update was ignored |
| Not JSON / invalid top level | <repository name>'s index is malformed |
| Higher `schema` | <repository name> uses a newer format; please update Vestra Player |
| Index too large | <repository name>'s index exceeds 4 MB; refused |
| Add repository: invalid address | Enter a repository address starting with https:// |
| Add repository: already exists | This repository has already been added |
| Add repository: limit reached | At most 16 repositories can be added |
| Add repository: read failed | Couldn't read this repository: <reason>. Check the address (the repository's root address or the full address of index.json) |
| Add a local http repository | (added to the confirmation dialog) This is a test repository on this machine, without encryption |

### 5.2 Download and install

| Situation | Text |
| --- | --- |
| Downloading | Downloading <name> <version> (42%) |
| Download failed | Download failed: <reason> |
| Size mismatch / exceeds the index size | The downloaded file does not match the repository's description (size); deleted |
| sha256 mismatch | The downloaded file does not match the repository's description (checksum); deleted |
| Link package over 64 MB | The plugin package exceeds 64 MB; download stopped |
| Link is not https | You can only install from https:// links |
| Redirect to http or too many | The download address is insecure or redirects too often; stopped |
| `validateArchive` → `invalid` | This is not a valid plugin package: <message> |
| Package content does not match the entry | The plugin package does not match the repository's description (<id/version/permissions>); not installed |
| `versionMismatch` | This plugin needs Vestra Player ≥ <x.y.z> (current <a.b.c>) |
| `unsupported` (same id as built-in) | “<id>” has the same id as a built-in plugin and cannot be installed |
| `reserved` (id starting with `vestra.`, player 0.4.1) | The host's original text: “<id>”: ids starting with vestra. are reserved for the player's built-in plugins and cannot be installed or loaded for testing |
| `conflict` | (goes to the replace confirmation dialog) |
| User cancels the confirmation | Not installed |
| `busy` | Plugins are being installed or cleaned up; try again later |
| `capability` (should not happen) | The Plugin Store lacks the “Manage plugins” permission |
| Install in progress | Installing <name>… (activating) |
| `committed` | Installed <name> <version> / Updated <name> to <version> |
| `failed` | Installation failed: <message> |
| `failedDegraded` | Installation failed; the previous version was restored |
| `cancelled` | Installation cancelled |
| `tooLate` | Can no longer cancel; the installation continues |
| Uninstall succeeded | Uninstalled <name> (data kept / data deleted) |
| Uninstall `unsupported` | Built-in plugins cannot be uninstalled or disabled (list refreshed; 2026-10-26 ruling) |
| Uninstall `missing` | This plugin no longer exists |
| Disk write failed | Couldn't write the download file: <reason> |

### 5.3 Other

| Situation | Text |
| --- | --- |
| README loading / failed | Loading the description… / Couldn't load the description: <reason> [Retry] |
| No README / CHANGELOG | The author provided no description / The author provided no changelog |
| Remote images blocked | N remote images blocked (unofficial source) [Show Images] |
| Unsupported link | This kind of link cannot be opened |
| No filter results | No plugins match |
| "Updates" is empty | All plugins are up to date |
| Revocation (see 3.10) | Disabled the revoked plugin “X” <version> |
| Headless mode | Log only; background checks and revocation work as usual |

---

## 6. Security review

### 6.1 What a malicious or tampered index can and cannot do

| Attack | Succeeds? | Defense |
| --- | --- | --- |
| Tamper with the official index (Pages, CDN or GitHub account compromised, signing private key not leaked) | No | Ed25519 signature verification; on failure the cache is used |
| Replay an old signed official index to hide revocations or updates | No | `generated` smaller than the largest accepted value is refused; seen versions and revocation entries only grow, and missing ones are also refused (3.4) |
| Delist a malicious plugin and `revoked: *` | Revocation takes effect | Revocation is judged by the installed plugin's source repository and does not depend on the index still listing it (3.10 item 5) |
| Revocation entries truncated above a limit | No | `revoked` > 1000 refuses the whole index |
| A third-party repository impersonates the official one (`official: true`, "official" in the name) | No | "Official" only means the built-in address + the built-in public key; the interface shows the host name |
| The index points `downloadUrl` at another package | No | `sha256` and `size` must match the entry; the package manifest's id, version and permissions must match the entry |
| The entry under-reports permissions, the package asks for more | No | Step 5 compares the permission sets; on mismatch it is not installed |
| Huge index / package / README, slow connections | No | All have size and time limits (3.2), aborted by stream counting |
| Zip bombs, path traversal, symbolic links | No (host) | `validateArchive` enforces limits during unpacking and refuses them (reference §12) |
| Replace a built-in plugin | No (host) | Any `install` with a built-in id is `unsupported` |
| Silently replace a same-id plugin installed from another source | No | The host returns `conflict`, and the store must ask the user |
| A third-party repository revokes and disables plugins of the official repository | No | Revocation only applies to the same source (3.10) |
| Display field injection (overlong, control characters, bidi overrides and zero-width characters, HTML) | No | Truncation; control characters and U+200B–200F, U+202A–202E, U+2066–2069, U+FEFF removed (so a name cannot look like another); shown as plain text; Markdown is parsed by the host and embedded HTML is dropped |
| `javascript:` and `file:` links in a README | No | Only `http(s)` is opened, and only on a user click; `system.openUrl` also only accepts `http(s)` |
| README remote image tracking | Official: yes (our own repository); third-party: no by default | Third-party images are not downloaded by default (2.2) |
| Tamper with or swap the README, changelog or icon (Pages / CDN compromised) | No (when the official index has hashes) | The size and sha256 must match the signed index, otherwise not shown and not cached (3.4) |
| A third-party index points the icon, README or changelog at another host (selecting the plugin reports to it) | No | For third-party repositories these assets are only read automatically from the same origin (scheme + host + port) as the index; when the README / changelog is elsewhere, the hint bar says "The description is on another website (host); not loaded automatically" + "Load"; when the icon is elsewhere it is not downloaded and the default icon is shown |
| Install or update without user confirmation | No | Every install has a confirmation dialog; automatic checks only count |
| Install a malicious plugin after the user confirms | **Yes** | Trust model (1.6 item 4): a plugin is a complete Node process, and permissions only limit host APIs. The only defenses are source review in the official repository, warnings for third-party and sideloaded plugins, and revocation |
| An installed malicious plugin modifies the store's cache / settings | Yes (any plugin can read and write the user's files) | The official cache is verified again before use; beyond that, out of what the store can defend |

### 6.2 Pinned public key, no TOFU

The official repository uses a built-in public key (ecosystem plan 1.6 item 3). TOFU's first connection has no protection, which is exactly when new users need protection most; a pinned key is fixed when the player is released, and since the store is a built-in plugin released with the player, a key change costs the same.

### 6.3 Key rotation

- The store trusts **all** `keys/*.pub.pem` in its package (`officialKeys()`; currently one, `index-signing.pub.pem`). `.sig` can have several lines (since plugin repository 68ff01b, one line per signing key); it is enough for any line to verify with any trusted public key (the code in 3.3). Both the plugin repository and the store already support multiple signatures.
- Planned rotation (plugin repository README "Key rotation"): ① the plugin repository adds `keys/index-signing-next.pub.pem` and starts dual signing (two lines in `.sig`); ② a player release puts both public keys in the store's `keys/` — only one file added and one store version released; ③ once all supported players know the new key, the plugin repository signs with the new key only, and a later store version deletes the old public key file.
- Private key leak: release a player version that trusts only the new key, and CI switches to the new key immediately. On players not yet upgraded, the store considers the new index's signature a mismatch and keeps using the last trusted cache (it sees no new updates or revocations); and since it still trusts the old key, **it will accept indexes the attacker signs with the leaked old key** (`generated` and `seen` only stop indexes older than the accepted one, not new high-sequence indexes the attacker signs). This is an inherent cost of the pinned-key approach, and the only remedy is to release quickly; the revocation list is signed by the same key, and there is no out-of-band revocation channel.

### 6.4 Other

- The signing private key exists only in the signing job of the plugin repository's CI (isolated in S3); tests always generate key pairs on the spot, and no private key is stored in any repository.
- If a GitHub account is compromised and can trigger the signing job, signing does not help (the signing job signs any index that passes the checks). This layer relies on GitHub environment protection rules and is outside the store's scope.
- The store stores no credentials and needs no `secrets`.

---

## 7. Cross-platform (workflow §1.5)

| Item | Approach |
| --- | --- |
| System commands | No system commands are called; links open with `system.openUrl`, network is Node `fetch`, verification `node:crypto`, files `node:fs`. `npm run portability` should report 0 places |
| Paths | Always joined with `node:path`; cache file names are hex digests (not affected by case-insensitive file systems or Windows reserved names); all data is under `dataDirectory`, so paths are short |
| Rename | On Windows `rename` fails when the target exists: download file names are random; the settings file uses `storage.json` (handled by the SDK) |
| Delete | On Windows the host may still have the just-verified file open: a failed delete is not reported, and `downloads/` is cleaned at startup |
| Line endings | Signatures are over raw bytes; the index is never written as text and read back. Test fixtures are generated in the tests, or written as base64 in `.ts`, unaffected by git's line ending conversion |
| Confirmation dialogs | macOS `NSAlert`; Windows task dialog — the "Uninstall" dialog with a check box becomes, on Windows, a confirmation followed by a second question whether to delete the data (reference §7); the wording reads well on both; other systems `unsupported`: the store shows "This system does not support confirmation dialogs; cannot install", and the confirmation is not bypassed |
| Drag and drop | Dropping on the video area is the same on both systems; modifier keys are not used |
| Certificates and proxies | Node `fetch` uses Node's bundled root certificates; it reads neither the system certificate store nor `HTTPS_PROXY`. On networks with a corporate proxy / self-signed root certificate, both systems fail to reach GitHub. This is runner-level configuration (Node 24's `--use-system-ca`, `NODE_USE_ENV_PROXY`) affecting all networked plugins; not solved in v1, listed as G5 (later) |
| Time display | `Intl.DateTimeFormat("zh-CN")`, independent of the system region format |

---

## 8. Test plan

### 8.1 Unit tests (`tests/*.test.ts`, SDK `TestHost`)

Dependency injection: core modules receive `{ fetch, keys, now, setTimer }`; `main.ts` passes the real values. The tests' `fetch` is a fake that answers by URL (it can return chunked streams, delays, redirects and errors); one additional test starts a server on 127.0.0.1 with `node:http` to verify stream aborts under the real `fetch`. No access to the internet.

In the implementation these were merged into 8 files (test key pairs are generated on the spot in `helpers.ts`; the network is always a fake `fetch` answering by URL, plus one test that starts a server on 127.0.0.1 with `node:http` and uses the real `fetch`):

| File | Coverage |
| --- | --- |
| `core.test.ts` | semver (all of the plugin repository's cases), player version comparison; the SPKI fingerprint of the built-in public key = the plugin repository's `keys/index-signing.pub.pem`; the live `index.json` + `.sig` of 2026-10-22 (`fixtures.ts`, base64) verify with the built-in public key, fail with one byte changed, and the live data selects bilibili / youtube 0.2.0; the various cases of test-key signatures (leading and trailing whitespace, non-base64, wrong length, another key, two signatures with one valid, key rotation); index parsing (unknown keys, per-entry dropping, truncation, sorting, duplicate ids, `schema: 2`, invalid top level); rollback detection; version selection (`apiVersion`, `minPlayerVersion`, revocation ranges); address normalization; plain-language permissions |
| `net.test.ts` | Exact size and sha256; abort and delete the file on 1 extra byte / missing bytes / checksum mismatch; `Content-Length` over the limit refused outright; stream without length aborted at the limit; every hop only https, the hop to http not requested, the 6th redirect refused, 5 allowed; 404, 304, total timeout and first-byte timeout, cancel, offline; real `fetch` + 127.0.0.1 server: local http only usable when allowed, an endless stream aborted at the limit |
| `repos.test.ts` | Opening the window reads and verifies, `User-Agent`; tampered index / another key / missing `.sig` keep the last trusted data and give the §5.1 text; replayed old index refused; offline restart uses the cache, a modified cache is no longer used; `304`; third-party repositories do not read `.sig`, `official` ignored, name with host name |
| `install.test.ts` | Official install (plain-language permissions in the confirmation dialog, download via 302, `source` is the repository address, download file deleted, buttons after install); declining the confirmation does not download; size / checksum mismatch; package with more permissions than the entry; another source → "Replace…" with one confirmation + `replace: true`, nothing installed when declined; a conflict the store did not know about → retry after a second confirmation dialog; same id as built-in; `versionMismatch`, `failedDegraded`, `busy`; same-repository update (confirmation dialog for new permissions, no rollback sentence), no Roll Back / Restore even when an older player still lists a previous version, no Install for versions older than the installed one in the Versions tab (and none from a stale window), uninstall with data deletion; third-party repository warning bar, unofficial confirmation dialog, README remote images blocked; README links only open http(s), "Code ↗" |
| `sideload.test.ts` | Drop two `.vxp` on the video area: window opens, a sideload warning for each, `source: file`; "Install from File…": cancel, invalid file, built-in id, replacing the same id installed from a repository; "Install from Link…": http refused, stop above 64 MB, permissions taken from the package, `source: link` |
| `revoked.test.ts` | Same-source hit → disable and merge into one notice dialog (fixed version); a second refresh does not disable again; red bar and buttons on the details page, cannot be installed again from the Versions tab; "Enable Anyway…" confirms the risk first, not disabled again after acceptance; re-enabled outside the store → confirmation afterwards, "Disable" disables again, "Keep Using" recorded as accepted; sideloaded, built-in and other-repository revocations not handled; when disabling is `busy`, retried in the next round |
| `store.test.ts` | Menu order 822 / `windows`, title with the update count (same source only; a higher version of the same id in another repository and built-ins do not count), OSD reported only once, count disappears after updating; automatic checks: no network at activation, once after 30 seconds (shortened in tests), then at the interval, no network when off; filter / search / tags / sort, incompatible and all revoked; README / changelog tabs, text when missing, "Show Images", "Retry" after a load failure; "Repositories…" opens the settings page, selection kept after closing and reopening the window |
| `settings.test.ts` | Settings page registration; adding: non-https, duplicate, read failure, confirmation dialog text, not added when declined; list after adding, automatic check switch; settings and cache kept after restart; after removal, plugins installed from it become local rows with a hint that they are no longer checked; limit of 16 |

`npm run plugin -- check|build|test store`, `npm run boundaries` and `npm run portability` must all pass before delivery.

### 8.2 Coordinator on-device acceptance checklist (step 2.5)

Preparation: packaged layout or development run, either is fine; this machine can reach GitHub; `zysilm.youtube` not installed; a local `.vxp` (output of `vestra-plugin pack`); a local third-party test repository (an `index.json` served on 127.0.0.1 with `npx http-server` or similar).

| # | Action | Observable result | Evidence |
| --- | --- | --- | --- |
| 1 | Context menu "Plugin Store…" | The window opens, two entries from the official repository (YouTube, Bilibili), status line "Official repository: updated at HH:mm" | Screenshot |
| 2 | Select YouTube | Details: logo placeholder icon, author, official repository, license; README rendered (Chinese, headings, lists); "Changelog" tab; 7 plain-language permissions in the right column | Screenshot |
| 3 | Click "Code ↗" and a link in the README | Opens in the default browser | Screenshot |
| 4 | "Install" | Confirmation dialog lists permissions → confirm → progress in the status line → "Installed YouTube 0.2.0"; the menu shows "Open ▸ YouTube Video…" and it works | Screenshot + menu |
| 5 | Disconnect the network and reopen the store | The list is still there, status line "Can't connect… showing the cache from …" | Screenshot |
| 6 | **Phase 4 leftover ①: install a `.vxp`** — "Install from File…" with a local package; then drop another `.vxp` on the video area | Both times a confirmation dialog (sideload warning) comes first, then the install; usable immediately after installing (hot enable); in the "Installed" filter the source is "Local file" | Screenshot |
| 7 | **Phase 4 leftover ②: cross-source replace and dormant data** — leave data in the YouTube installed in #4 (sign in or settings); pack a `.vxp` with the same id using `vestra-plugin pack`, then "Install from File…" | State "Installed from another source" → replace confirmation dialog (text mentions data is kept) → after replacing, the new instance's data is empty (a different data folder in the store's details); then click "Replace…" on the official YouTube in the store → after switching back the original data is still there | Screenshot + data folder in the store's details |
| 8 | **Phase 4 leftover ③: Plugin Workshop install** — the workshop generates and installs the counter plugin | `workshop.counter` appears under "Installed" in the store, source "Plugin Workshop", "only on this machine" hint, no update check; its menu works | Screenshot |
| 9 | Built-in plugins | Under "Installed", built-in plugins are marked "Built-in", with no uninstall / replace button | Screenshot |
| 10 | Add the local test repository (127.0.0.1) on the settings page | Confirmation dialog (unofficial + local without encryption) → an "Unofficial: …" row appears in the list; README remote images blocked, and "Show Images" can be clicked | Screenshot |
| 11 | Add a new version of an installed plugin in the test repository and refresh | Menu title "Plugin Store (1 update)…", OSD; appears under the "Updates" filter; update → only "Uninstall…" remains (no Roll Back); the Versions tab offers no older version | Screenshot |
| 12 | Add the installed version to `revoked` in the test repository and refresh | The plugin is disabled + notice dialog (reason, fixed version); red bar "This version has been revoked… The store has disabled it." and "Enable Anyway…" on the details page; click it → risk confirmation dialog → after confirming the plugin runs, a refresh no longer disables it, and the red bar changes to "You confirmed the risk…". Another time: enable it from outside the store (another plugin with `plugins.manage`) → the store afterwards asks "has been revoked. Keep using it?"; choosing "Disable" disables it again | Screenshot |
| 13 | A version with `minPlayerVersion: "9.0.0"` in the test repository | That version is marked "Incompatible" and cannot be installed | Screenshot |
| 14 | ~~Disable the store (Plugin Manager)~~ **Superseded 2026-10-26:** the store is a built-in and cannot be disabled; the Plugin Manager is gone | — | — |
| 15 | Windows (done together with S6) | Repeat 1, 4, 6, 7 on Windows; paths, confirmation dialogs (the second uninstall dialog is a yes / no dialog) | Screenshot |

---

## 9. API gaps

### 9.1 Conclusion: v1 has no blocking gaps

Each capability the store needs, compared one by one:

| Need | Existing approach | Basis |
| --- | --- | --- |
| HTTP requests with size and time limits | Node `fetch` + stream counting + `AbortSignal` | Start of the reference: plugin processes can use the network; the S4 §5.3 Markdown download does it the same way |
| Download to a temporary location | `dataDirectory/downloads/` + `node:fs` | reference §3.4 |
| Ed25519 signature verification | `node:crypto` `verify(null, …)` | Tested with the live index and the built-in public key (fails after tampering); `check-boundaries` only restricts relative paths and SDK imports, `node:` built-in modules are not restricted |
| Player version | `plugin.app.version` | reference §3.5 |
| Install `.vxp`, source, replace | `packages.validateArchive`, `candidate.install({ source, replace })` | reference §12 |
| Update / uninstall / disable | `install` (same source), `uninstall`, `setEnabled` | reference §12 |
| Installed list with sources and built-in flag | `source`, `builtin` of `packages.list()` / `onChange` | reference §12 |
| Install by drop | `drops.provide({ extensions: ["vxp"] })` (video area) | reference §6.7; S4 §10 states explicitly "install by drop is done by the store with the existing `ui.drop`" |
| Choose a file | `dialogs.openFile` | reference §7 |
| Confirm | `dialogs.confirm` (with check box, `destructive`) | reference §7 |
| Open links | `system.openUrl` | reference §16.5 |
| README / CHANGELOG | `ui.markdown` (`base`, `remoteImages`) + `MarkdownView` | reference §6.1 |
| Details page icon | Download to the data directory + `ui.image` | reference §6.1 |
| Persistent settings | `storage.json` + `settings.page` | reference §16.4, §6.9 |
| Notifications | `osd.show` + count in the command title | reference §6.5, §4 |
| Scheduled background checks | `setTimeout` / `setInterval` in the plugin process, cleaned up in `onDispose` | reference §3.3 |
| Link input widget | `UrlInputRow` | reference §6.4 |

### 9.2 Optional improvements (not blocking, alternatives exist)

| # | Gap | Why it is needed | Minimal API shape | Plugin-side alternative |
| --- | --- | --- | --- | --- |
| G1 (not this round, decision 7) | `ImageRef` cannot be put into lists / structs | VS Code-style lists have a plugin icon on each row, which helps recognition | Allow `ImageRef` as a struct field inside array properties (the host recognizes `$vestraImage` when building the model in `prop_value`, the same as for images in the Markdown model): `window.set({ rows: list.map(p => ({ name: p.name, icon: plugin.ui.image(path) })) })`, and in Slint `struct Row { icon: image, … }`. Check and cache rules unchanged | **Yes**: the list shows no icons, only the details page does (adopted in v1). No effect on function |
| G2 (not this round, decision 7) | Tool windows do not receive drops | Users are used to dropping a `.vxp` on the store window (VS Code supports dropping on the extensions view) | `Window.onDrop(listener)`, same signature as `Panel.onDrop` (needs `ui.drop`); the host forwards drops to tool windows that have a listener | **Yes**: drop on the video area (adopted in v1), with a hint in the window; the "Install from File…" button |

### 9.3 Contract clarification (documentation, not a new API)

| # | Question | Current state | Wanted |
| --- | --- | --- | --- |
| G3 (resolved) | Whether installing a **lower version** from the same source is allowed ("Install Other Version") | The host's `install_instance` in `package_manager.rs` only compares source identity (`current.source.identity() == source.identity()` keeps the instance), and the whole `packages.install` path (`stage` → `start_operation`) never compares version numbers; the old current version becomes `previousVersion` | Decision 7: allowed, as the host currently behaves. reference §12 has one added sentence (with step 2.4). **Superseded 2026-10-26 (Q2):** the store no longer installs an older version from the same source |

### 9.4 Later (not in v1, no request)

| # | Content | Note |
| --- | --- | --- |
| G4 | Extension filter for `dialogs.openFile` | Only a convenience; `validateArchive` refuses other files. Several plugins (subtitles, playlist, screenshots) may want it; it will be requested together once there is a second requester |
| G5 | System root certificates and proxies for plugin processes | Parameters the runner passes when starting Node (`--use-system-ca`, `NODE_USE_ENV_PROXY`), affecting all networked plugins; go through §4 once a user actually reports it |
| G6 (known, 2026-10-28) | The workshop's `check` tool validates on every check, and each passing validation takes one of the host's 32 recent candidates (`MAX_CANDIDATES`) | A store install whose token was pushed out by 32 newer validations meanwhile gets `stale` ("unknown validation token"), which the store reports as a failed install; installing again validates again. Possible only while the workshop runs 32 checks between the store's validation and its install (the confirmation dialog). No change unless a user meets it; a fix would be a validation that keeps no candidate (a check-only flag) |

---

## 10. Implementation plan

### 10.1 Files (after implementation)

```text
src/plugins/store/
  manifest.json        0.1.0, minPlayerVersion 0.2.0, icon assets/icon.png
  README.md            for users (also shown in the store's details)
  CHANGELOG.md
  keys/index-signing.pub.pem   unchanged copy of the plugin repository's keys/ (shipped in the package; tests check its SPKI fingerprint)
  assets/icon.png      256×256, the store's own icon, also the placeholder when the details page has no icon
  main.ts              activate: real fetch, built-in public key, clock                              29
  store.ts             state and window properties, menu count, automatic checks, drops, README / CHANGELOG loading   813
  repos.ts             address normalization, cache, conditional requests, verification, rollback detection   324
  index-format.ts      index parsing and per-entry validation                                       213
  catalog.ts           repositories × install library → rows and states (§2.3), version selection (§3.5)   187
  installer.ts         repository / link / file install, confirmation dialogs, conflict and replace, uninstall   404
  revocations.ts       §3.10                                                                         187
  settings-page.ts     settings page                                                                 126
  net.ts               get(): https, manual redirects, timeouts, stream limits, sha256, writing files   221
  icons.ts             download and cache of details page icons                                     112
  labels.ts            plain-language permissions, source texts, dates                              104
  semver.ts            ported from the plugin repository's semver.mjs                                85
  signing.ts           built-in public key, verifyIndex (multiple signatures)                        34
  ui/store.slint       main window                                                                   477
  ui/settings.slint    settings page (440 wide)                                                      116
  ui/parts.slint       Chip, Choice, Check, Tone                                                     192
  tests/               8 test files + helpers.ts + fixtures.ts (§8.1)                                1800
```

About 2840 lines of TS (excluding tests), about 790 lines of Slint. No npm dependencies.

### 10.2 Order

| Step | Content | Done when |
| --- | --- | --- |
| 1 | **Vertical slice**: menu "Plugin Store…" → read and verify the official index → list → details (no README) → confirm → download and verify → install | YouTube can be installed from the real official repository (unit tests + coordinator on-device #1, #4) |
| 2 | README / changelog / versions tabs, icon, permission column, source code link | #2, #3 |
| 3 | Cache and offline, refresh scheduling, update count and menu title, update, uninstall, install another version | #5, #11 |
| 4 | Sideloading: file, drop, link; replace confirmation | #6, #7, #8 |
| 5 | Revocation | #12 |
| 6 | Settings page and third-party repositories, remote image policy | #10, #13 |
| 7 | Wording and edge cases (headless, Windows confirmation dialog differences), README | All unit tests, `check`, `boundaries`, `portability` |

All open questions have been decided (§11).

---

## 11. Coordinator decisions (2026-10-22, step 2.2)

The first draft listed 6 open questions here; all have been decided:

| # | Question | Decision | Where |
| --- | --- | --- | --- |
| 1 | Separate plugin or merged into the Plugin Manager | A separate built-in plugin `vestra.store`. **Superseded 2026-10-26:** the Plugin Manager was removed and the store is the only management interface | §1.3 |
| 2 | Menu position | Top level "Plugin Store…", right next to "Plugin Workshop…" (order 822); the title carries the count when updates are available | §1.2 |
| 3 | Whether the store offers uninstall and roll back | Yes; the Plugin Manager keeps its buttons, and both call the same API. **Superseded 2026-10-26:** there is no rollback, and the Plugin Manager is gone | §1.3, §2.3 |
| 4 | The user enables a revoked plugin again manually | Disable automatically once and keep showing a warning; enabling again requires confirming the risk first, after which it is no longer forcibly disabled | §2.3, §3.10 |
| 5 | Trust for third-party repositories | v1 only marks them "Unofficial" with a warning, no configured public keys, no TOFU; README remote images not loaded by default | §2.2, §4 |
| 6 | Automatic check frequency | Once 30 seconds after activation, then every 24 hours, on by default, can be turned off on the settings page | §3.6 |
| 7 | G1, G2, G3 | G1 and G2 not this round (no list icons; drop on the video area + "Install from File…"); G3 allowed as the host currently behaves, reference §12 updated | §9 |
| 8 | Monotonic `generated` index field, multiple signatures | Put on the to-do list, not blocking S5 (the store is already compatible: unknown keys ignored, multiple signatures accepted) | §3.4, §6.3 |

## 12. Implementation notes (details that differ from the first draft)

- **Revocation**: per decision 4, "Enable Anyway…" and the after-the-fact confirmation were added (§3.10 item 3); the check also runs on every install library change.
- **Automatic checks**: per decision 6, the "≥ 6 hours since the last check" condition was removed (§3.6).
- **Window not open when dropping**: the window is opened first, then the confirmation dialog is shown; the dialog already has the package's name, version and permissions, so the details are not first "shown from the manifest"; after installing, the newly installed row is selected.
- (Superseded 2026-10-26: there is no rollback.) **Roll back has no confirmation dialog** (the first draft's §2.5 did not list one either): a rollback can be undone with "Update"; when rolling back to a revoked version, the revocation check handles it as usual.
- **URL fields in the index** (`iconUrl`, `readmeUrl` and so on) that are not https are only set to empty, without dropping the whole plugin; only an invalid `downloadUrl` drops the version. When the repository itself is local (`http://localhost` and so on), its own local http links are usable — which is why the local test repository in the coordinator acceptance checklist #10–#13 can install plugins.
- **Fetch again when the signature does not match**: the index and `.sig` are two requests, and if the index is updated exactly in between, the first pair does not match; only when a second pair still does not match is it judged a failure.
- **Same id installed from another source**: the list has two rows, one for the repository entry (state "Installed from…", only "Replace…"), and one for the locally installed instance (appears under the "Installed" filter, can be uninstalled).
- (Superseded 2026-10-26: there is no rollback.) **Roll back button**: when the previous version is newer than the current one (after installing an old version or rolling back), it reads "Restore to x", with the result "Restored X to x" (on-device acceptance S3).
- **Data note for replace**: `PluginInfo` cannot see dormant instances, so the store cannot know whether this source was installed before; the confirmation dialog therefore says "If it was installed from this source before, its data from then is used again; otherwise it uses new, empty data" (S5).
- **Right column information**: version, installed, updated, source, id, license, Code ↗.
- **Settings page**: id `store.repos`, category "Plugins", order 20.
- **Icons** (store 0.2.1): list rows and the details show the manifest `icon` of installed plugins (copied into `<data>/icons/`) first; otherwise rows use the repository's `iconUrl` through `ui.remoteImage` and the details the same address downloaded and checked against the index's size and sha256 into `<data>/icons/`; the store's own icon when neither exists. G1 was closed by player 0.5.1.
- **Slint**: `ui/store.slint` compiles with 0 warnings under Slint 1.18.1 (`fluent-dark`, library `vestra`); `ui/settings.slint` and `ui/parts.slint`, compiled on their own, only give notes of the kind "Exported component … doesn't inherit Window", the same as the settings page / part files of the keyboard shortcuts plugin (by contract, a settings page exports a non-Window component).
