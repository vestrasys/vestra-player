# Plugin Store

Browse the plugins in the official Vestra Player repository and install, update, roll back or uninstall them.

## Entries

- Right-click menu: Plugin Store…. When updates are available it reads Plugin Store (2 updates)….
- Drop a `.vxp` plugin package on the video area to install it.
- Options ▸ Plugins ▸ Plugin Store: third-party repositories and the automatic update check.

## Features

- **List**: the plugins of the official repository and of the repositories you added, and the plugins installed on this machine. Search (name, author, tags), filter by All / Installed / Updates, pick a tag, sort by name or by update time.
- **Details**: description, changelog, version history and the permissions the plugin needs (in plain words; the ones to watch are amber).
- **Install and update**: every install lists the permissions and asks first. Updates come only from the plugin's own repository; after an update you can roll back to the previous version. The Versions tab installs another version.
- **From a file or a link**: Install from File… and Install from Link… in the window, or drop a `.vxp` on the video area. Such plugins are not reviewed by any repository, and the store does not check them for updates.
- **Replace**: when the same plugin is already installed from somewhere else, the store asks before replacing it. The replacement starts with new, empty data; the old data is kept and comes back when you reinstall from the original source.
- **Revocations**: when a repository revokes a version, the store disables it and tells you why; its page keeps a warning. You can confirm the risk and enable it again, and the store will not disable it again.
- **Automatic check**: 30 seconds after the player starts, then every 24 hours; it only counts updates and never installs on its own. It can be turned off on the settings page.

## Security

- The official repository's index is digitally signed, and the store checks it with a built-in public key; an index that does not verify is not used, and the last trusted copy stays. A package is installed only when its size and checksum match the index.
- The only official plugins are the built-in ones that come with the player, with ids starting with `vestra.`. The plugins in the official repository are third-party plugins, written by their authors and reviewed by the repository; the list marks them Official repository. An id starting with `vestra.` cannot be installed from any repository or package.
- Plugins of third-party repositories are marked Unofficial: <repository>; they and sideloaded plugins come with a warning before installing. A third-party README does not load remote images until you click Show Images.
- A plugin is a complete program: permissions only limit what it can do with the player, not reading and writing files or using the network. Only install plugins from authors you trust.

## Limits

- Built-in plugins are updated with the player and cannot be installed, replaced or uninstalled here; disable or reload them in Settings ▸ Plugin Manager….
- The list shows no plugin icons (only the details do); dropping a `.vxp` on the store window does nothing, drop it on the video area.
- Behind a company proxy or with self-signed certificates the repositories may be unreachable.

## Translations

Texts are English in the source; `locales/zh-Hans.po` has the Simplified Chinese (`vestra-plugin i18n extract`, `i18n check`). Sentences built at run time are templates in `text.ts`, handed to the player as text values (`plugin.i18n.text`), which it translates in the windows, the OSD and the confirmations alike.
