# Changelog

## 0.2.4

- The permission `rendering.manage` (Vestra Player 0.5.13, SDK 1.5.35) is shown in words: "Choose GPU or software rendering and restart the player", in amber like the other risky permissions, in the install and update confirmations and the plugin details. Before, it read "Unknown permission: rendering.manage".

## 0.2.3

Needs Vestra Player 0.5.10.

- The store has a button on the bottom bar (a shopping bag), right of the AI button and left of the sidebar toggle. A click opens the store like Plugin Store… in the menu; its tooltip is the menu title, with the number of updates when there are any. In a narrow window the button goes before the sidebar toggle does; the menu entry stays either way.

## 0.2.2

- The details always show a plugin of the list. Switching tabs keeps the selected plugin when the new tab lists it, otherwise selects the tab's first plugin; a search or tag that hides the selected plugin does the same, and an empty list leaves the details empty until it fills (an index read, a refresh that adds matches), then selects its first plugin. A tab that has no plugin with the chosen tag shows all its plugins again (Tags: All). Before, Available could show the details of an installed plugin that was not in its list.
- Installing, updating or uninstalling keeps the tab: when the plugin leaves the list (an install leaves Available, an update leaves Updates), the plugin in its place is selected, as when its repository drops it. A plugin installed from a file or a link, and the revocation notice's Open Plugin Store, still show the plugin on its own tab.
- The list scrolls the selected plugin into view after a tab, search, tag or sort change.

## 0.2.1

Finishes the plugin center. Needs Vestra Player 0.5.1.

- Each row shows the plugin's logo. An installed plugin uses the icon from its own package, so it shows offline too; a plugin from a repository uses the repository's icon, loaded only once its row is shown; otherwise the store's own icon. Logos are decoded small (twice their drawn size).
- Reload now works for running plugins too, built-ins included, and starts a failed one again. The row says Working… until the plugin has restarted, then the status line says whether it runs or why it failed. A plugin being installed or reloaded, or one disabled in the meantime, gets a clear message instead.
- A running plugin that was refused something another plugin holds (a panel position, Previous/Next, a file type) carries a Warning tag; hover for the reason, and the details say it in full. This used to be shown only in the Plugin Manager.
- Plugins loaded for development (`--plugin`) are listed under Installed with a Dev tag and can be disabled and enabled there; disabling one lasts only until the player quits, and they cannot be uninstalled from the store.
- Disabling still asks no question.
- After a refused uninstall, or when a plugin turns out to be gone, the store reads the plugin list again, so the rows match the player.
- The details' icon of an installed plugin now comes from its own package first.
- The ⚙ menu and the rows' … menus act again: picking an entry did nothing, because the menu closed before reading which entry was picked.

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
