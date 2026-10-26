# Plugin Manager

See every plugin of the player; disable, enable, reload or uninstall them.

## Entry points

- Menu `Settings ▸ Plugin Manager…`: opens the options window on the Plugin Manager page.
- Also reachable from the Plugins category on the left of the options window.

## Features

- **Plugin list**: each plugin's name (id), version, source (repository, link, local file, Plugin Workshop, built-in, development; without the address), state (running, starting, disabled, failed, stopped) and data size. The list updates by itself when plugins are installed, uninstalled or disabled.
- **Details**: for the selected plugin, the failure reason (for a running plugin, a warning instead, for example that its panel position is taken by another plugin, that its panel is not shown, or that previous/next is provided by another plugin; the state in the list is then drawn in the accent colour), source (repository, link, local file, Plugin Workshop, built-in; with the address for repositories and links), package folder (for a built-in plugin, inside the player's installation), data folder and the capabilities it uses. Long paths are shortened; hover to see all of it.
- **Disable / Enable**: disabling asks first. Its menus, shortcuts, windows and effects are removed at once, and the open media keeps playing. It stays disabled after the player restarts; enable it here at any time.
- **Reload**: stops and starts the plugin again, to recover one that went wrong.
- **Uninstall…**: after confirming, removes the plugin from the player. Tick "Also delete its settings and data" to remove those too; by default they are kept, and a reinstalled plugin picks them up again.
- **Open Data Folder**: opens the folder where the plugin keeps its settings and data in the file manager.

## Limits

- Plugin Manager cannot disable or uninstall itself.
- Built-in plugins (installed with the player) can only be disabled, not uninstalled; they are updated with the player.
- An update replaces the plugin; the player keeps no earlier version. To go back to an older version of a plugin, uninstall it and install that version's `.vxp`.
- Development-loaded plugins can only be disabled and enabled, and disabling lasts only for the current run.
- A plugin that never saved any data has no data folder to open. Each plugin's data size is measured for at most 2 seconds; after that it shows "—".
- Installing new plugins and browsing the store are not done here.

## Languages

English source text; Simplified Chinese in `locales/zh-Hans.po`. The page's words are in `ui/plugins.slint` (`@tr`); TypeScript passes codes and data.
