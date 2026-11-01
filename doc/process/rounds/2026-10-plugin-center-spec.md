# Plugin center: product specification (UI polish round, step 1)

Written by the coordinator on 2026-10-26 for `2026-10-ui-polish.md` step 1. Status: **approved by the coordinator (2026-10-26). The user delegated the decision ("decide yourself").** Implementation is next.

## Scope

Keep the store's existing window, colours, controls and layout (`src/plugins/store/ui/store.slint`; today's screenshot in the round record). Change only what the user named:

1. The store becomes the **only** place where plugins are managed. The Plugin Manager (`vestra.plugins`) and "Settings ▸ Plugin Manager…" are removed.
2. Categories go in a **tab bar**.
3. Repository management and manual install go behind **one icon menu**.
4. Each list row has an **inline action button** like the VS Code extension list, plus the plugin's **logo**.

User rulings that apply here:
- Plugin management is **install, uninstall, disable/enable**, and nothing else.
- **Built-ins can be neither uninstalled nor disabled.**
- There is no rollback, no older-version install and no Undo.
- The store **works offline**.

No redesign beyond this. In particular: no new colours, no new component styles, and no change to the details pane (Description / Changelog / Versions tabs, the metadata column, Markdown).

## Window

```text
┌ Plugin Store ───────────────────────────────────────────────────────────────┐
│ [ Installed (18) ]  [ Available (2) ]  [ Updates (1) ]              (⟳) (⚙) │  ← tab bar; refresh + gear icons
│ [ Search names, authors, tags…                ] [Tags: All ▾] [Sort: Name ▾] │
│ You can also drop a .vxp plugin package on the player's video area…         │
├────────────────────────────────┬────────────────────────────────────────────┤
│ [logo] Bilibili 视频     0.3.0 │ [logo] Bilibili 视频                       │
│        zysilm · Official       │        zysilm · Official repository        │
│        粘贴链接播放…  [Uninstall]│        粘贴链接播放 Bilibili 单个视频       │
│                          [ … ] │ [Uninstall…] [Disable]                     │
│ [logo] Album Cover…      0.1.7 │ Description | Changelog | Versions │ meta  │
│        Vestra Player·Built-in  │  …Markdown…                        │ …     │
│        Shows the album…        │                                    │       │
└────────────────────────────────┴────────────────────────────────────────────┘
│ status line (offline / last refresh / result of the last action)            │
```

### Tabs (replace the All / Installed / Updates filter buttons)

| Tab | Shows | Count |
| --- | --- | --- |
| **Installed** (default when opened from the menu) | every installed plugin: built-ins, then third-party; disabled and failed ones too | number installed |
| **Available** | plugins from the repositories that are not installed | number available; "—" offline without a cache |
| **Updates** | installed third-party plugins with a newer compatible version | number; the tab is emphasised when > 0 (as the menu item's count is today) |

The search, Tags and Sort controls apply within the current tab. The tab bar uses the same tab style as the details pane's Description/Changelog/Versions tabs; no new component.

### Toolbar icons (replace the four text buttons)

- **⟳ Refresh** (icon button, tooltip "Refresh"): same action as today's Refresh.
- **⚙ menu** (icon button, tooltip "More"): a menu with
  - Install from File…
  - Install from Link…
  - Repositories…
  - separator, then Store Settings… (opens the store's existing settings page)

The icons come from the player's existing icon set if suitable ones exist (`doc/product/identity/icons-v-line.json`). Otherwise an icon-button capability is requested from the maintainer per §4.1.

### List rows

> **Superseded by Store 0.2.9** (2026-11-01): an installed plugin's actions now keep one fixed order, Update / Disable·Enable / Uninstall…; the row button is the first of them, the `…` menu holds the rest plus Reload (not while disabled) and Open Data Folder, and the details page shows the same buttons, with Reload last only for a failed plugin. A plugin disabled before an update stays disabled. The table below is the original spec; see [store design §1.3 and §2.3](../../architecture/store-design.md).

```text
[logo 32] Name                         version
          Author · Source / Built-in
          Short description (one line, ellipsis)      [Primary] [ … ]
```

- **Logo**: the plugin's own icon (manifest asset, or the repository index icon). If it has none, the store's default icon. **Offline, installed plugins use the icon from their own package** (fixes the open item).
- **Primary button** (existing Button component, small): exactly one per row, by state.

| Row state | Primary button | `…` menu |
| --- | --- | --- |
| Available | Install | — |
| Installed third-party, enabled | Uninstall… | Disable, Reload, Open Data Folder |
| Installed third-party, disabled | Enable | Uninstall…, Open Data Folder |
| Installed third-party, update available | Update | Disable, Uninstall…, Reload, Open Data Folder |
| Installed third-party, failed | Reload | Disable, Uninstall…, Open Data Folder; the row shows the reason |
| Built-in | none; the "Built-in" tag stays | Reload, Open Data Folder |
| Operation running | the button shows progress and is disabled | — |

- A disabled plugin's row is dimmed and carries a "Disabled" tag. A failed one carries a "Failed" tag in the warning colour, with the reason on hover and in the details pane.
- Confirmations (install, uninstall, uninstall with data) keep today's dialogs and texts.

### Details pane

Unchanged layout. The action buttons under the title mirror the row: the primary button plus Disable/Enable. The metadata column gains what the Plugin Manager used to show: **Status** (Running / Disabled / Failed: reason), **Package** (path), **Data folder** (path, with Open), **Data size**, and the **Capabilities** list, which today shows as "Permissions" (keep the existing label).

### Removed

- The `vestra.plugins` plugin: its menu item "Settings ▸ Plugin Manager…" and its settings page. Its tests move to the store wherever the behaviour moves.
- The sentence "Disable, reload, open the data folder: Settings ▸ Plugin Manager…" under the details pane.

### Offline (user requirement)

- Opening the store, the Installed tab and its details, uninstall, disable/enable, reload, open data folder and install from a `.vxp` file all work offline.
- Available and Updates show the last known state. Without a cache they show a calm note, "Can't reach the repositories. Showing nothing until you're online; click ⟳ to retry.", in the normal text colour, not the warning colour.
- The refresh on open uses a short timeout (for example 5 s for headers), so that a blackholed network doesn't keep the list spinning for 15–30 s.
- Installed plugins' details offline: the package's own README/CHANGELOG (store 0.1.8 already does this), with a small note "From the installed version".

## Acceptance (coordinator, Windows; screenshots in en, zh-Hans, de)

1. Open the store from the menu: the Installed tab is selected and lists 17 built-ins, with no primary button and only Reload/Open Data Folder in their `…` menu.
2. Install zysilm.bilibili from Available. It appears in Installed with Uninstall… as its button. Disable it from `…`: the row dims and the button becomes Enable. Restart: it is still disabled. Enable it, then uninstall it with data.
3. The ⚙ menu has Install from File…, Install from Link…, Repositories… and Store Settings…. Install a `.vxp` through it.
4. Offline (the official repository made unreachable): steps 1–3 work except installing from a repository. Available/Updates show the calm note within about 5 s.
5. The Plugin Manager is gone from the menu and from Options. No text anywhere says built-ins can be disabled.
6. The screenshot tool renders the store window in all three languages without clipping or overflow.

Status (store 0.2.1, 2026-10-26, checked by the user on Windows): row logos, built-in rows and VestraTube 0.4.0 install pass. The ⚙ menu entries did nothing (also the rows' `…` menus: `MenuButton` closed its popup before reading the picked entry); fixed in 0.2.1. Coordinator recheck 2026-10-27 (scripted input, screenshots): ⚙ ▸ Install from Link… shows the link box; a built-in's `…` ▸ Reload restarts it ("Reloaded …", status Running); ⚙ ▸ Install from File… installs VestraTube 0.4.1 through the file picker and the confirmation dialog. **Host defect found**: after a plugin restarts (Reload, or an install that replaces a running plugin) the main window comes to the front and the store tool window drops behind it; the store is not closed, only hidden. For the maintainer. **Step 4 (offline) not yet run**: the coordinator still owes it, with the repository made unreachable without taking the machine off the network (for example a hosts-file or firewall rule for the repository host, or a profile whose repository URL points at a dead local port).

## Who does what

- **Store author:** everything inside `src/plugins/store/`.
- **Plugins author:** retire `vestra.plugins`, and hand over the tests and texts that move.
- **Maintainer:**
  - remove `vestra.plugins` from the built-in list (`stage-plugins`, `builtin.json`);
  - answer any §4.1 requests (an icon button, a tab bar component, reading plugin logos);
  - check that an installed plugin's own icon can be read from its package.
- The authors work in their own worktrees and merge in separate PRs where nothing depends on another change (user rule, 2026-10-26: keep agents decoupled).
