# Changelog

## 0.3.3

- The window resizes as a window should (reported: it could not be made taller than about half the screen but could be squeezed to nothing, and the cards jumped while it was resized). The hidden element that measures translated labels was an unplaced child of the window, so Slint added its layout limits to the window's: the maximum height came out 0, under the minimum of 480, and Slint swapped the two, giving the system a maximum height of 480 and no minimum. The window also opened 480 high instead of 640. Now it is at least 600 × 480 (wider when the tabs need it in the interface language), has no maximum but the screen, and opens at 920 × 640.
- Resizing keeps the cards in view: when the number of columns changes, the card at the top left stays in the top row, which starts at the top of the grid (the scroll position used to stay in pixels, so other cards slid into view). A narrower window drops a column as soon as a card would be under 264 pixels wide; a wider one adds a column only 24 pixels past that point, so a width that trembles during a drag does not flip the grid back and forth.
- Who must change: nobody. Plugin authors: a hidden measuring element directly in a `Window` needs `x` and `y` (any value), or its limits become the window's.

## 0.3.2

- The "how to open" convention puts the sidebar first (owner request 2026-11-21): a plugin with a sidebar tab writes `Sidebar: <tab> tab` (with its key when it has one), and adds its next way in after `; ` only while the line stays within 48 characters, for example `Sidebar: VestraTube tab; Menu: Open ▸ VestraTube`. README and `open-line.ts` say so; the fallback for a plugin without the line still reads menus and palette rows only, since the command table has no tabs. A test checks that every built-in plugin that mounts a sidebar tab (today the Playlist) names the sidebar first. Nothing the store shows changes.
- No automatic repository check in a temporary window (guest windows, "Play in New Window", player 0.10.3): the plugin reads `plugin.app.temporary` (SDK 1.10.1). A temporary window runs the main window's plugins read-only: the player refuses installs, updates, uninstalls and enabling or disabling there (`unavailable`), and the store shows that error. Refresh still works. A main window is unchanged. Who must change: nobody.

## 0.3.1

- A new tab **Not installed** right after All (as Steam and Synaptic name it): the repositories' plugins that this player can install and that are neither installed, built in nor loaded with `--plugin`, one card per repository as under All, with its count in the title. Entries that cannot be installed (Incompatible, every version revoked, a reserved `vestra.` id) are left out: the tab is what you could still add, and All keeps them with the reason. It follows search, Category, Sort and "Show built-in plugins" like the other tabs; empty, it says "There are no more plugins to install", or, without any index, that the repositories cannot be reached. The tab is remembered by name, so a tab kept by 0.3.0 (Installed, Updates) opens as before.
- The window's minimum width grows past 600 when the four tabs need it in the interface language (Spanish, Japanese), so Refresh and ⋯ stay in the window; the search box shrinks to 120 first.
- A card's "✓ Installed" is green: a darker green on the light palette, the video plugin's green on the dark one (the theme has no green token; both read at 4.5:1 or more against the panel).
- The accent Update button: "Update to x" on a plugin's page is drawn like the card's Update; hover and press darken it; disabled, it looks like any disabled button (white text on the light palette's pale accent could not be read).
- A card's ⋯ menu closes when the grid changes under it, so it can no longer act on the plugin that took that card's place.
- "Replace with x from y…" shows "Installing… n%" on the installed plugin's card, as Install and Update do.
- Translations of the tab title and the empty-tab text: zh-Hans by hand; de, es, fr, ja and ko by machine, marked `x-machine` for review.

## 0.3.0

- Store v3 (design store-v3 rev. 2, owner rulings 2026-11-15): the window is one grid of cards instead of a list with the details always beside it. The grid takes as many columns as fit (two to five, by a Slint binding on the grid's width; a `ListView` of rows, so only the visible rows are built); the window is at least 600 × 480 (was 760), 920 × 640 at first.
- A card: icon (drawn 40, decoded at 80), name, "by <author>" with "· Unofficial" for another repository or the source of a plugin from no repository, the introduction, the "how to open" line, the state in words (✓ Installed, Update available, Disabled, Failed, Revoked, Comes with the player, Incompatible, Cannot be installed, Development load, Installed from …) and at most one button, Install or Update. Every line is one line, cut with "…" by the window; the tooltip has the whole introduction. Version, tags, badges and the row's ⋯ manage menu are gone from the list.
- A click on a card opens the plugin's page in the same window; ← All Plugins, Esc or the mouse's back button returns to the grid as it was (tab, search, category, scroll). The grid has no selection any more: a tab, search or sort change no longer moves any details. The page has the header and the buttons [Update to x] [Disable | Enable] [Uninstall…] [Reload], a ⋯ menu with Open Data Folder and "Replace with x from y…", the notices, "Where to find it", and the tabs **About** (the README) and **Details** (facts, permissions, Changelog / Versions). It follows its plugin through install, update and uninstall, and closes when the plugin leaves the list.
- Reload is now a button on the page of every running or failed plugin (was: in the row's ⋯ menu, and on the page only for a failed plugin or one with an update). Built-in plugins' pages offer Reload, and Open Data Folder in the ⋯ menu.
- "How to open": a new convention in the manifest `description` (no manifest field, owner ruling 2026-11-15): a last line starting with `→ ` and at most 48 characters after it is shown on the card and under "Where to find it", and the description is shown without it. Without one, an installed plugin's card shows its own menu entry or command palette row from `commands.list`. See the README.
- The Tags drop-down is replaced by Category, a fixed list (Picture, Sound, Subtitles, Online video, Library and playlists, Tools, Games, Other) matched from the plugins' tags; tags still feed the search. An empty result offers Clear Search. The ⚙ button is now ⋯; the "drop a .vxp" hint left the window (the README keeps it).
- Store v4 (design store-v4, owner rulings 2026-11-15), in the same release: the top is one 31px strip (the library Tabs at their own height, the search box, Refresh, ⋯) with a rule under it, then a filter row with the check box **Show built-in plugins (n)** (moved out of the ⋯ menu; same setting and default), Category and Sort. The detail page keeps the strip, with ← All Plugins in it.
- Installed cards act in place (store-v4 §3): a ⋯ (Manage) with Disable or Enable and Uninstall… (asks first), placed before the card's text button and opening upward near the window's bottom; Enable is the text button of a disabled card; Update wins over Enable and is drawn in the accent colour with white text (a Store-local button from Theme tokens). Not on cards: Reload, Open Data Folder, Replace, Enable Anyway…. A revoked card offers Uninstall… only, a development load Disable or Enable only, a built-in nothing. A disabled card with an update says "Update available".
- While a plugin installs from its repository its card shows "Installing… n%" instead of the Install button. A search or category with no result says how many hidden built-in plugins would match and offers Show Built-in Plugins.
- New text translated by machine (marked `x-machine`; zh-Hans by hand). Who must change: nobody. Plugin authors may add the "→ " line to their description.

## 0.2.22

- A plugin the player set aside at start (listed failed, without a package directory) no longer offers Disable or Reload, which the player refused with "no plugin"; its row and details offer Uninstall… (and Update or a reinstall from its repository). With player 0.10.1 that uninstall removes it completely, after which it can be installed again. The same holds for a failed row whose package could not be read this run (retried at the next start). Who must change: nobody.

## 0.2.21

- Plugins the player lists without a directory (installed records it set aside at start: player 0.9.0 sets aside packages built before it, whose manifest still has `capabilities`) each show their own name. Formerly their display fields were cached by directory and version alone, both empty, so every such row showed the first one's name. Nothing is read from the working directory for them either.

## 0.2.20

- A package not from the official repository (another repository, a file, a link) is no longer silent about its permissions: its install and update confirmations say **It is not from the official repository, so its permissions were not checked.** (user ruling 2026-11-13). A file or a link: after "Plugins may read your files or use the network.". Another repository: after its warning, which says files and network already; its update and reinstall confirmations: "Current: x." then both sentences. Formerly a file or a link said only the files-and-network sentence, and another repository's update only "Current: x.".
- Another repository's own `scan` is its claim, so it is shown nowhere: not in its install confirmation (formerly its categories followed the warning), its update confirmation and the window's notice "The new version needs new permissions: …" (formerly compared from its scans), nor the window's Permissions list and heading.
- The official repository is unchanged from 0.2.19: the categories of the scan in its signed index, "It uses none of the player's sensitive features." when it found none, no list without a scan. Nothing is scanned on this machine. New text translated by machine (marked `x-machine`; zh-Hans by hand). Who must change: nobody.

## 0.2.19

- Manifest capabilities are gone (SDK 1.9.0, player 0.9.0): `capabilities` is removed from manifest.json. The store declared none of the two that changed how a plugin ran (`events`, `playback.read`), so `activate()` needs no `plugin.player.subscribe()` or `read()` call.
- A repository index version's `capabilities` is now optional legacy data: signed indexes from before SDK 1.9.0 still carry it, so it is accepted in any form (or absent) and never read. Formerly a version without a valid list was dropped from the index. The install check that a package declares exactly what its index entry lists is gone, since packages declare nothing; the package must still match the entry's id and version, and its size and checksum.
- Unchanged: what the store tells you about a plugin comes only from the index `scan`, with no list without a scan (as in 0.2.18). Who must change: nobody; a repository may drop `capabilities` from new index versions.

## 0.2.18

- What the store tells you about a plugin now comes only from the repository's scan of its code, never from the capabilities the package declares (SDK 1.8.0, plan A1/A6; user ruling 2026-11-13). A repository's index may carry the scan per version (`versions[].scan.uses`, the `uses` of `vestra-plugin scan`); the store shows its categories in plain words: player settings and the playback engine, other plugins, clipboard, credentials, camera, microphone, screen, notifications; a word the store does not know is named as an unknown permission. Without a scan (an index from before it, a `.vxp` from a file or a link) there is nothing to show.
- Install confirmations: with a scan, "It will be able to:" and the scanned categories, or **It uses none of the player's sensitive features.** when the scan found none; without a scan, no list. Formerly they listed every declared capability.
- Every install confirmation says **Plugins may read your files or use the network.**: the official repository's and a package from a file or a link (an unofficial repository's warning said it already, and is unchanged).
- The update confirmation and the window's notice "The new version needs new permissions: …" compare scans only: both versions scanned, the categories the new one adds; only the new one scanned, every category it uses; the new one not scanned, nothing. Formerly they named the capabilities declared anew.
- The Permissions list in the store window shows the scanned categories of the version shown (all drawn as ones to watch); without a scan the list and its heading are not shown. Formerly it listed the declared capabilities.
- The wording for the new capability `mpv` (SDK 1.8.0), risky, is in the store's table as the player's: **Control the playback engine (mpv) directly: send its commands and change its options**.
- New texts are translated by machine (marked `x-machine`; zh-Hans by hand). Unchanged: what a plugin may call (the player still gates the API by the declared capabilities), the check that a package declares what its index entry says, and how plugins are installed. Who must change: nobody; a repository that wants the categories shown adds `scan` to its index versions.

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
