# Plugin Store

Browse the plugins in the official Vestra Player repository; install, update, uninstall, disable and enable them. The store is where plugins are managed.

## Entries

- Right-click menu: Plugin Store…. When updates are available it reads Plugin Store (2 updates)….
- Bottom bar: the shopping-bag button, between the AI button and the sidebar toggle (Vestra Player 0.5.10). Its tooltip is the menu title.
- Drop a `.vxp` plugin package on the video area to install it.
- Options ▸ Plugins ▸ Plugin Store: third-party repositories and the automatic update check.

## Features

- **Tabs**: All (every plugin from the repositories, installed or not, and the installed plugins no repository lists, such as a `.vxp` you installed or one made in the Plugin Workshop; each row shows its state and its buttons; an installed plugin has one row, whose … menu can replace it with a repository's version), Installed (built-in plugins first, then the others, disabled and failed ones too) and Updates, with their counts. The store opens on the tab you used last (All the first time). Search (name, author, tags, in the interface language and in English), pick a tag, sort by name or by update time, within the tab. Under the drop-downs, "Hide built-in plugins" (on by default, kept across restarts) leaves the plugins that come with the player out of the lists and the counts; untick it to see them. Refresh and the ⚙ menu (Install from File…, Install from Link…, Repositories…, Store Settings…) are at the top right.
- **Rows**: the plugin's logo (an installed plugin's own icon, offline too; otherwise the repository's icon or the store's), and one control in the same place at the bottom right, as in VS Code's extension list: Install for a plugin not installed, Update while a newer version exists, otherwise a ⋯ Manage button whose menu keeps one order, Disable or Enable, Uninstall…, Reload (not while disabled), Open Data Folder, then "Replace with x from y…" where a repository offers it. Right before the version, a grey ✓ marks an installed plugin, or Disabled (the row dimmed) or Failed says why it does not run; the second line says the rest (Update → x.y.z, Revoked, Built-in…). The details keep their buttons, [Update to x] [Disable | Enable] [Uninstall…], so disabling or enabling a plugin changes only the words, never the places (Reload last, for a failed plugin). Built-in plugins have no Disable or Uninstall; their menu offers Reload and Open Data Folder. A Warning tag marks a running plugin that was refused something another plugin holds (a panel position, Previous/Next, a file type); hover for the reason, the details say it in full.
- **Details**: description, changelog, version history and the permissions the plugin needs (in plain words; the ones to watch are amber); for installed plugins also the status, package folder, data folder and data size.
- **Disable and enable**: no question asked; a disabled plugin stays off after a restart and after an update, a reinstall or a replacement from another source (the status line says "(still disabled)"), and its row is dimmed. Reload restarts a running plugin, built-ins included, or starts a failed one again.
- **Development loads** (`--plugin`): listed under Installed with a Dev tag. They can be disabled and enabled, but disabling lasts only until the player quits, and they cannot be uninstalled here.
- **Offline**: the Installed tab, details, uninstall, disable, enable, reload and installing a `.vxp` work without a network. Opening the store waits at most 5 seconds for a repository; All and Updates then show the last known state; without one, All shows the installed plugins with a note and Updates shows a note.
- **Install and update**: every install lists the permissions and asks first. Updates come only from the plugin's own repository. The Versions tab installs a newer version than the installed one; to go back to an older version, uninstall the plugin and install that version's `.vxp`.
- **From a file or a link**: Install from File… and Install from Link… in the ⚙ menu, or drop a `.vxp` on the video area. Such plugins are not reviewed by any repository, and the store does not check them for updates.
- **Replace**: when the same plugin is already installed from somewhere else, the store asks before replacing it. The replacement starts with new, empty data; the old data is kept and comes back when you reinstall from the original source. A disabled plugin stays disabled after the replacement.
- **Revocations**: when a repository revokes a version, the store disables it and tells you why; its page keeps a warning. You can confirm the risk and enable it again, and the store will not disable it again. Updating it to a fixed version enables it again, unless you disabled it yourself after confirming the risk.
- **Automatic check**: 30 seconds after the player starts, then every 24 hours; it only counts updates and never installs on its own. It can be turned off on the settings page.

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
