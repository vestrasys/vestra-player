# Changelog

## 0.2.18

- The install confirmations list what matters instead of every capability (SDK 1.8.0, plan A1/A6). When the repository's index carries the capability scan of the version (`versions[].scan.uses`, the `uses` of `vestra-plugin scan`), they list the scanned categories in plain words: player settings and the playback engine, other plugins, clipboard, credentials, camera, microphone, screen, notifications; a word the store does not know is named as an unknown permission. Without a scan (an index from before it, a `.vxp` from a file or a link) they list only the risky capabilities the package declares, and any the store does not know. With nothing to list they say **It uses none of the player's sensitive features.**
- Every install confirmation says **Plugins may read your files or use the network.** before that list: the official repository's and a package from a file or a link (an unofficial repository's warning said it already, and is unchanged).
- The update confirmation compares the scanned categories of the installed and the new version when the index has both, and otherwise the declared capabilities as before (all of them, not only the risky ones). The notice in the store window, "The new version needs new permissions: …", compares the same way, so the two never disagree.
- The permission list names the new capability `mpv` (SDK 1.8.0), risky: **Control the playback engine (mpv) directly: send its commands and change its options**.
- New texts are translated by machine (marked `x-machine`; zh-Hans by hand). Unchanged: the Permissions list in the store window, which goes by the declared capabilities; what is installed and how. Who must change: nobody; a repository that wants the categories shown adds `scan` to its index versions.

## 0.2.17

- The permission list names the new capability `video.motion` (SDK 1.7.19): **Smooth motion with the GPU's frame-rate converter**, translated by machine (marked `x-machine`; zh-Hans by hand). Who must change: nobody.

## 0.2.16

- The permission list names the new capability `media.sources` (SDK 1.7.8): **Remember videos you add and export from them later**, translated by machine (marked `x-machine`). Who must change: nobody.
- Plugin Store… moves into the right-click menu's Tools ▸ (path `["@tools", title]`, the anchor added with the short menu; user ruling 2026-11-11): Tools ▸ ends with Vestra Player AI…, Plugin Store…, Command Palette…, then About. The title with the update count ("Plugin Store (2 updates)…") keeps the new path, as do the id, order 822, group `windows`, the palette category and the bottom-bar button. Who must change: nobody.

## 0.2.15

- The install and update confirmations end with the permission list (or the "Current: …" line); the closing sentence "A plugin is a complete program: the permissions above only limit what it can do with the player, not reading and writing files or using the network." is gone (user ruling 2026-11-08). The same grey line under Permissions in the store window is gone too. The warnings for unofficial repositories and sideloaded packages are unchanged. Who must change: nobody.

## 0.2.14

- Plugin Store… has a category for the player's command palette (Ctrl+Shift+P, player 0.7.6), which names a command "Category: Title": **Plugins**, so the palette reads "Plugins: Plugin Store…" (or "Plugins: Plugin Store (2 updates)…"), next to Vestra Player AI…. Translated like the player's word for plugins (machine, marked `x-machine`). No `home`: the command opens its own window. Who must change: nobody. Title, id, menu path, button and order are unchanged.
- Requires player 0.7.6 (`minPlayerVersion`): older players refuse a command with a category.

## 0.2.13

- Code only: the store window handle is named `storeWindow` (was `window`). Who must change: nobody. No behaviour change.

## 0.2.12

Needs Vestra Player 0.6.0.

- Each row in the list has one control in the same place at the bottom right, as in VS Code's extension list: Install for a plugin not installed, Update while a newer version exists, otherwise a ⋯ Manage button (tooltip and accessible name "Manage"). Its menu keeps the fixed order: Disable or Enable, Uninstall…, Reload, Open Data Folder, then "Replace with x from y…" under All. Formerly an installed row showed a Disable or Enable button and a … button, next to the not-installed rows' single Install button.
- The "Installed" label is gone: a grey ✓ right before the version marks an installed plugin (accessible name "Installed"); a disabled plugin's row is dimmed and says "Disabled" there instead, a failed one "Failed". "Update → x.y.z", Revoked, Built-in and the other states stay on the second line.
- The details keep their buttons ([Update to x] [Disable | Enable] [Uninstall…]). A row with an update offers only Update; disable, uninstall or reload it from the details, which then also offer Reload (after Uninstall…, while it runs) and "Replace with x from y…" when another repository has a version of it (under All).

## 0.2.11

Needs Vestra Player 0.6.0.

- The tabs are now All · Installed · Updates, All first (as in GNOME Software, KDE Discover, the JetBrains Marketplace and VS Code). All lists every plugin from every repository, installed ones included, and the installed plugins no repository lists (a `.vxp` from a file or a link, a plugin made in the Plugin Workshop). Each row shows its state and keeps its buttons in their fixed places: Install for a plugin not installed; Update (while a newer version exists), Disable or Enable, Uninstall… for an installed one. Search, the tag, the sort and "Hide built-in plugins" apply to All as they did to Available, which All replaces. The titles read "All (n)", "Installed (n)" and "Updates (n)"; Updates is still in the accent colour while updates wait.
- All shows one row per installed plugin: a plugin installed from a `.vxp`, the Plugin Workshop or another repository that a repository also lists, and a built-in a repository lists (with "Hide built-in plugins" off), has only its installed row. Its details say "Also in <repository>", and its … menu ends with "Replace with <version> from <repository>…", which asks as Replace… did. A plugin that is not installed still has one row per repository.
- Without any repository index (offline, nothing cached), All still shows the installed plugins and counts them, with a calm line under the list saying the repositories can't be reached (Available showed "—" and an empty list). Installed, uninstall, disable and installing a `.vxp` still need no network.
- The store remembers the last tab across restarts too (in its settings.json; before, only while it ran). The first open is All.

## 0.2.10

Needs Vestra Player 0.6.0.

- Works with players that keep a disabled plugin disabled across an install (after 0.6.3) as well as with older ones, which turn a plugin on when they install it. The status line now says what the player reports once the install is done (the plugin list is read again; it is also read again when an install starts, so a plugin disabled elsewhere a moment earlier stays disabled), instead of guessing from the state before the install: "(still disabled)" appears exactly when the plugin ended up disabled.
- Updating a revoked version the store disabled to the fixed version enables the fix again on every player: the store turns it on after the install (on a newer player the install kept it disabled, and the status line said "Updated" while it stayed off). The status line waits (up to 5 seconds) until the fix has started; when the player refuses to enable it, or it fails to start, it says "Updated X to y, but couldn't enable it: …" with the reason.
- Replacing a disabled plugin with the same plugin from another source (a repository, a file or a link) keeps it disabled, as an update or reinstall does; formerly an older player left the replacement running.

## 0.2.9

Needs Vestra Player 0.6.0.

- Buttons keep their places when a plugin is disabled or enabled (as on VS Code's extension page): only Disable and Enable swap their words. The details show [Update to x.y.z] (while a newer version exists) [Disable | Enable] [Uninstall…], plus Reload last for a failed plugin; formerly Enable jumped to the first place after Disable. A development load shows [Disable | Enable] (and Reload when failed).
- A row's button is the first of the same order (Update, else Disable or Enable) and its … menu the rest in a fixed order: Disable or Enable, Uninstall…, Reload, Open Data Folder. Reload is left out for a disabled plugin (the player does not reload one). Formerly the button was Uninstall… for a running plugin and Enable for a disabled one, and Reload for a failed one.
- A disabled plugin with an update offers Update as well (formerly only a revoked one did).
- A plugin you disabled stays disabled after an update or reinstall (as in VS Code): the player turns a plugin on when it installs it, so the store turns it off again right away and says so ("Updated X to y (still disabled)"). Formerly an update enabled it. A revoked version the store disabled is the exception: updating it to the fixed version enables it, as before.

## 0.2.8

Needs Vestra Player 0.6.0.

- A "Hide built-in plugins" check box under the tag and sort drop-downs, on by default (also for people who used the store before). When on, the plugins that ship with the player (they can be neither uninstalled nor disabled) are left out of every tab: their own rows under Installed and a repository's entries for them under Available. The tab titles count what the tabs list ("Installed (3)" leaves the built-ins out while they are hidden), and the tag list offers only tags of the listed plugins. Search and the tag combine with it. Unticking shows them as before, built-ins first under Installed. The choice is kept in the store's settings and survives a restart.
- Hiding the selected plugin selects the list's first plugin, or nothing when the list is empty, as a search or a tag does; a tag only built-ins have goes back to "Tags: All". The check box is reached with Tab after the drop-downs and toggled with Space.
- With only built-in plugins installed and hidden, the Installed tab says so ("No plugins installed besides the built-in ones, which are hidden") instead of "No plugins installed yet". A built-in shown for you (`select`) while hidden turns the option off first, and keeps the tab, tag and search when they already list it.

## 0.2.7

Needs Vestra Player 0.6.0.

- Both tab rows (Installed / Available / Updates and Description / Changelog / Versions) are the player's tab strip (`Tabs`): with the focus on a row, Left/Right (and Home/End) switch tabs, and screen readers announce the row as a tab list. They stay 22px high on the window's background; Updates is still in the accent colour while updates wait.
- The settings page's "Check for updates automatically" takes the check box's new state instead of flipping the setting, so a repeated or late event cannot turn it back.

## 0.2.6

Needs Vestra Player 0.6.0.

- The store uses the player's shared controls (`@vestra/widgets.slint`): the tag and sort drop-downs, the tabs (Installed / Available / Updates and Description / Changelog / Versions, still 22px high) and the settings page's check box look like the other windows'. Tabs are at least 74px wide, the current one has the short accent underline, and the pointer lights a plate; Updates is still in the accent colour while updates wait. The drop-downs and the check box can be reached with Tab and worked with the keyboard (arrows, Space, Enter, Esc); a click does not take the focus from the search box.
- The tag drop-down's first entry ("Tags: All") now comes translated from the plugin; the drop-down lists show at most 11 rows and scroll past that, as before.
- The "⋯" menus (the row menu and the More menu at the top right) stay the store's own: the player's menu (`Panel.showMenu`) is only for docked panels, not tool windows.

## 0.2.5

- The permission `video.upscaling` (Vestra Player 0.5.16, SDK 1.5.37) is shown in words: "Upscale the video with the GPU's video upscaler", in the install and update confirmations and the plugin details. Before, it read "Unknown permission: video.upscaling".

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
