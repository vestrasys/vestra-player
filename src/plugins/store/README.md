# Plugin Store

Browse the plugins in the official Vestra Player repository; install, update, uninstall, disable and enable them. The store is where plugins are managed.

## Entries

- Right-click menu: Tools ▸ Plugin Store…, between Vestra Player AI… and Command Palette…. When updates are available it reads Plugin Store (2 updates)….
- Bottom bar: the shopping-bag button, between the AI button and the sidebar toggle (Vestra Player 0.5.10). Its tooltip is the menu title.
- Drop a `.vxp` plugin package on the video area to install it.
- Options ▸ Plugins ▸ Plugin Store: third-party repositories and the automatic update check.

## Features

- **The grid** (Store v3, header and card actions Store v4): one window of cards, as many columns as fit (two to five; the window is at least 600 wide, wider when the tabs need more in the interface language). On top a strip with the tabs, the search box (name, author, tags, in the interface language and in English), Refresh and the ⋯ menu (Install from File…, Install from Link…, Repositories…, Store Settings…); under it the filter row: "Show built-in plugins (n)", Category and Sort (name or update time).
- **Tabs**: All (every plugin from the repositories, installed or not, and the installed plugins no repository lists, such as a `.vxp` you installed or one made in the Plugin Workshop; an installed plugin has one card), Not installed (the repositories' plugins that this player can install and that are neither installed, built in nor loaded with `--plugin`: what you could still add; incompatible, revoked and reserved entries stay under All with their reason), Installed (built-in plugins first, then the others, disabled and failed ones too) and Updates, with their counts. The store opens on the tab you used last (All the first time). Built-in plugins are left out of the grid and the counts until "Show built-in plugins (n)" is ticked in the filter row (kept across restarts; n is how many there are).
- **Category**: a fixed list, Picture, Sound, Subtitles, Online video, Library and playlists, Tools, Games and Other, matched from each plugin's tags (`open-line.ts`); a plugin whose tags match none is in Other. An empty result offers Clear Search, which clears the search and the category, and, when hidden built-in plugins would match, says so and offers Show Built-in Plugins.
- **Cards**: the plugin's icon (an installed plugin's own icon, offline too; otherwise the repository's icon or the store's), its name and "by <author>" (with "· Unofficial" for another repository, or where a plugin from no repository came from), its introduction (the punchline), the "how to open" line (below), and at the bottom its state in words (✓ Installed in green, Update available, Disabled, Failed, Revoked, Comes with the player…) with at most one text button, the likely next step: Install, Update (in the accent colour) while a newer version exists, or Enable on a disabled card. An installed plugin that is not built in also has a ⋯ (Manage) before that button: Disable or Enable, and Uninstall… (asks first); a revoked one Uninstall… only, a development load Disable or Enable only. While a plugin installs from its repository, its card shows "Installing… n%" instead of its button. Every line is one line; the window cuts what does not fit with "…", and the card's tooltip has the whole introduction (or why it failed, or what a running plugin was refused). A click anywhere else on the card opens its page.
- **A plugin's page**: ← All Plugins (or Esc, or the mouse's back button) returns to the grid as it was. The header (icon, name, author and source, introduction), then the buttons in fixed places, [Update to x] [Disable | Enable] [Uninstall…] [Reload], so disabling or enabling changes only the words (Reload leaves while disabled), and a ⋯ menu with Open Data Folder and, under All, "Replace with x from y…" where another repository offers it. Built-in plugins have Reload and Open Data Folder only. Then the notices, "Where to find it" (the how-to-open line in full), and two tabs: **About** (the README) and **Details**: version, source, id, license, code, and for installed plugins status, package folder, data folder and data size; the permissions the official repository's scan found (in plain words, amber; none without a scan, and none for other repositories); the changelog and the version history. The page follows its plugin through an install, an update or an uninstall; a plugin that leaves the list (a sideloaded one uninstalled) closes it.
- **Disable and enable**: no question asked; a disabled plugin stays off after a restart and after an update, a reinstall or a replacement from another source (the status line says "(still disabled)"), and its card is dimmed. Reload restarts a running plugin, built-ins included, or starts a failed one again.
- **Development loads** (`--plugin`): listed under Installed, their card says "Development load". They can be disabled and enabled, but disabling lasts only until the player quits, and they cannot be uninstalled here.
- **Offline**: the Installed tab, details, uninstall, disable, enable, reload and installing a `.vxp` work without a network. Opening the store waits at most 5 seconds for a repository; All, Not installed and Updates then show the last known state; without one, All shows the installed plugins with a note, and Not installed and Updates show a note.
- **Install and update**: every install asks first. From the official repository it lists what the plugin's code was found to use (the categories of the repository's capability scan); without a scan it lists nothing, since what a package declares about itself is not shown. Any other package (another repository, a file, a link) is said to be not from the official repository, with its permissions not checked, on install and on update; another repository's own scan is not shown. Updates come only from the plugin's own repository. The Versions tab installs a newer version than the installed one; to go back to an older version, uninstall the plugin and install that version's `.vxp`.
- **From a file or a link**: Install from File… and Install from Link… in the ⋯ menu, or drop a `.vxp` on the video area. Such plugins are not reviewed by any repository, and the store does not check them for updates.
- **Replace**: when the same plugin is already installed from somewhere else, the store asks before replacing it. The replacement starts with new, empty data; the old data is kept and comes back when you reinstall from the original source. A disabled plugin stays disabled after the replacement.
- **Revocations**: when a repository revokes a version, the store disables it and tells you why; its page keeps a warning. You can confirm the risk and enable it again, and the store will not disable it again. Updating it to a fixed version enables it again, unless you disabled it yourself after confirming the risk.
- **Automatic check**: 30 seconds after the player starts, then every 24 hours; it only counts updates and never installs on its own. It can be turned off on the settings page.

## How to open: a convention in the description

A card's second line says where the plugin is found once it is installed. It comes from the plugin's `description` in its manifest (owner ruling 2026-11-15; there is no manifest field for it): **a last line that starts with `→ ` (U+2192 RIGHTWARDS ARROW and a space), with at most 48 characters after it**.

```json
"description": "Saves, saves as or copies the current frame …\n→ Menu: Tools ▸ Snapshot"
```

- Write where the user finds the plugin, in the player's words, with ` ▸ ` between menu levels and without a trailing "…": `Menu: Tools ▸ Snapshot`, `Sidebar: Playlist tab (F6)`, `Adjustments and Effects ▸ Picture (Alt+1)`, `Options (F5) ▸ Keys and Mouse`, `Command palette (Ctrl+Shift+P): Window`, `Automatic: album cover and idle logo`. A plugin that is always visible (the control bar) needs none.
- Translations keep the arrow and the 48 characters (the line is read from the description in the interface language, so the repository's index carries it to cards of plugins not yet installed). Up to about 40 characters show in full on the narrowest card.
- The store shows the line on the card and in full under "Where to find it", and shows the description without it. Anything else (no arrow, a longer line, the arrow not on the last line) leaves the description as it is and the card's line empty.
- Without the line, an installed plugin's card shows its own menu entry, "Menu: <path>", or else its command palette row, "Command palette: <Category>: <Title>", read from the player's command table (`commands.list`, which only built-in plugins such as the store may read). A plugin not installed shows nothing.

## Security

- The official repository's index is digitally signed, and the store checks it with a built-in public key; an index that does not verify is not used, and the last trusted copy stays. A package is installed only when its size and checksum match the index.
- The only official plugins are the built-in ones that come with the player, with ids starting with `vestra.`. The plugins in the official repository are third-party plugins, written by their authors and reviewed by the repository; the list marks them Official repository. An id starting with `vestra.` cannot be installed from any repository or package.
- Plugins of third-party repositories are marked Unofficial: <repository>; they and sideloaded plugins come with a warning before installing. A third-party README does not load remote images until you click Show Images.
- Only install plugins from authors you trust.

## Limits

- Built-in plugins are updated with the player and cannot be installed, replaced, uninstalled or disabled. The store cannot reload itself.
- Dropping a `.vxp` on the store window does nothing, drop it on the video area.
- Behind a company proxy or with self-signed certificates the repositories may be unreachable.

## Translations

Texts are English in the source; `locales/` has German, Spanish, French, Japanese, Korean and Simplified Chinese (`vestra-plugin i18n extract`, `i18n check`). Sentences built at run time are templates in `text.ts`, handed to the player as text values (`plugin.i18n.text`), which it translates in the windows, the OSD and the confirmations alike.
