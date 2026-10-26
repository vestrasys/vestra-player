# Changelog

## 0.2.0

The store is now where you manage plugins (plugin center).

- Tabs Installed (the first one), Available and Updates, with their counts, replace the All / Installed / Updates filters. Installed lists the built-in plugins first, then the others, including disabled and failed ones.
- The toolbar's text buttons became a refresh button and a ⚙ menu: Install from File…, Install from Link…, Repositories… and Store Settings….
- Each row has one button for its state (Install, Uninstall…, Update, Enable or Reload) and a … menu with Disable or Enable, Uninstall…, Reload and Open Data Folder. A disabled plugin's row is dimmed and tagged Disabled; a failed one is tagged Failed, with the reason on hover. Built-in plugins have no button, and their menu offers only Reload and Open Data Folder: they can be neither disabled nor uninstalled.
- The details add Status, Package, Data folder (click to open) and Data size, and the buttons under the title include Disable. The pointer to the Plugin Manager is gone.
- Offline: opening the store waits at most 5 seconds for a repository; Available and Updates then say calmly that the repositories can't be reached, and the status line is no longer in the warning colour. An installed plugin's own README or changelog is marked "From the installed version". The details icon of an installed plugin comes from its own package when the repository's icon can't be downloaded.
- Names, punchlines, descriptions and tags show in the interface language: for installed plugins from the player's translation of their manifest, for the others from the repository index's new optional `translations` (the language, then its base language, then English). Search and tags also match the English text.

## 0.1.8

- Built-in plugins can be neither uninstalled nor disabled: the store no longer sends you to the Plugin Manager to disable one, and a refused uninstall reads "Built-in plugins cannot be uninstalled".
- The repositories page says how to keep the store offline: turn off automatic update checks; the store then goes online only when you open it or click Refresh.
- Offline, an installed plugin's README and changelog come from its own package instead of an error, and the Updates filter says it can't connect instead of "All plugins are up to date" when no repository index has ever been read.

## 0.1.7

- The uninstall confirmation's check box reads "Also delete its settings and data?", with a question mark, in every language.

## 0.1.6

- No rollback (the player no longer keeps a previous version): the Roll Back / Restore action is gone, the confirmations no longer say you can roll back or restore, and the Versions tab no longer installs a version older than the installed one from the same repository. To go back to an older version, uninstall the plugin and install that version's `.vxp`.
- A refused install while the player is busy reads "Plugins are being installed or cleaned up; try again later".

## 0.1.5

- Version bump only, so that players that had already installed 0.1.4 pick up the changes shipped since then; no change in behavior.

## 0.1.4

- English source text with a Simplified Chinese translation (`locales/zh-Hans.po`); the Chinese interface reads as before. Needs Vestra Player 0.4.3.
- Sentences built at run time go to the player as text values (SDK 1.5.11), so the permission lists, the paragraphs of the install confirmations, the revocation reasons, the download failure reasons and the update count in the menu title are translated too; the windows no longer need a Slint word table.

## 0.1.3

- Ids starting with `vestra.` are reserved for the player's built-in plugins: a repository listing one shows Cannot be installed with the reason and nothing is downloaded; a package the player refuses as `reserved` shows the player's reason.
- Plugins of the official repository are marked Official repository (was Official), so they are not taken for official plugins; the only official plugins are the built-in ones.

## 0.1.1

- The settings page's add-repository row no longer covers the warning below it; wrapped notes have looser line spacing (1.25), so lines no longer crowd with the system font.

## 0.1.0

- First version: the official repository (signature checked), third-party repositories, install / update / roll back / uninstall, install from a file or a link, drop to install, revocations, automatic update check.
