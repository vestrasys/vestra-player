# Changelog

## 0.1.6

- No rollback (the player no longer keeps a previous version): the Roll Back / Restore action is gone, the confirmations no longer say you can roll back or restore, and the Versions tab no longer installs a version older than the installed one from the same repository. To go back to an older version, uninstall the plugin and install that version's `.vxp`.
- A refused install while the player is busy reads "Plugins are being installed or cleaned up; try again later".

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
