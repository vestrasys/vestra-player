# Changelog

## 0.1.19

- About Vestra Player shows the author's copyright, `© 2026 Ziyang Song`, under the version (user ruling 2026-11-27), in the secondary text colour; Copy Info copies it as the third line. A name, not translated. Who must change: nobody.

## 0.1.18

- `subscribe()` and `player.onChange` are gone (SDK 1.11.0): the overlay (Tab) and the short information (Shift+Tab) open a state stream and listen to `window.changed` (for the video width the short lines are cut to) only while one of them shows, beside their existing 1-second refresh; Playback Properties streams the state while its window is open, where it used a listener of its own. With nothing shown the plugin holds nothing and may be stopped while idle. What is shown is unchanged. Who must change: nobody.

## 0.1.17

- The hidden elements that measure translated labels in the About and Properties windows are placed (`x`, `y`), so their layout limits stay out of the window's. Nothing changes today; the same unplaced element made the Plugin Store's maximum height 0, so its size limits swapped (Store 0.3.3). Who must change: nobody.

## 0.1.16

- No automatic update check in a temporary window (guest windows, "Play in New Window", player 0.10.3): the plugin reads `plugin.app.temporary` (SDK 1.10.1). Check for Updates… still works there. A main window is unchanged. Who must change: nobody.

## 0.1.15

- The description's last line, starting with `→ `, says where to open the plugin; Store 0.3.1 shows it on the plugin's card, also before install. Description text and its translations only. Who must change: nobody.

## 0.1.14

- Manifest capabilities are gone (SDK 1.9.0, player 0.9.0): `capabilities` is removed from manifest.json. `activate()` now starts with `plugin.player.subscribe()` (was `events`) and then `await plugin.player.read()` (was `playback.read`), the calls that replace the two declarations that changed how it ran; the others need nothing. Behaviour is unchanged, except the ruled start-up order: the player's `window.state` read now comes before the state requests. Who must change: nobody.

## 0.1.13

- Menu (the short menu, user ruling 2026-11-11, SDK 1.7.12): About Vestra Player… (F1) moves from the top level of the right-click menu to the end of the player's Tools ▸ (`["@tools", "About Vestra Player…"]`); id, order 880, group `settings-main`, key and category are unchanged. No other info command is in the menu (Playback Info, Brief Info, Playback Properties… and Check for Updates… stay palette-only). Who must change: nobody.

- Code only (SDK refactor ruling 11, built-in sweep pilot): what the About and Playback Properties windows and the Updates settings page show lives in `plugin.state` (SDK 1.7) instead of variables followed by a manual `plugin.update()`: the plugin list, renderer and status line (About), the tab, status, stats, File and System pages (Playback Properties), and the update state, error, progress, installer and saved settings (`Updater`). A write re-runs only the bindings that read the field; the one-second stats poll recomputes only the bindings that read the statistics (the SDK still adds a safety pass about once a second while the window is open, so full passes are fewer, not gone). All 9 manual update calls are gone; the download progress is still written at most ten times a second. Who must change: nobody. No behaviour change.

- Menu sweep (menu redesign §4): Playback Info (Tab), Playback Properties… (Ctrl+F1) and Check for Updates… leave the right-click menu and are in the command palette only ("Info: …", "Help: Check for Updates…"), with ids, keys and the checked state unchanged; About Vestra Player… (F1) stays at the top level and joins the settings group (`settings-main`) with Options…, so one separator group holds Command Palette…, Options… and About. Who must change: nobody.

- Every command has a category for the player's command palette (Ctrl+Shift+P, player 0.7.6), which names a command "Category: Title": **Info** (Playback Info, Brief Info, Playback Properties…) and **Help** (About Vestra Player…, Check for Updates…), so the palette reads "Help: Check for Updates…" like other players' Help menus. "Info" uses the translations of the plugin's tag; "Help" is new, machine translated in all six languages (`x-machine`). Check for Updates… names About Vestra Player… as its place (`home`): the About window's Updates row has the same action. Who must change: nobody. Titles, ids, menu paths, keys and order are unchanged.
- Requires player 0.7.6 (`minPlayerVersion`): older players refuse a command with a category.

## 0.1.12

- Code only: the window handles are named `aboutToolWindow` (About, `ui/about.slint`) and `propertiesToolWindow` (Playback Properties, `ui/properties.slint`) (were `window`). Who must change: nobody. No behaviour change.

## 0.1.11

- J10 Check for updates (restored by the user's ruling of 2026-11-01): the player looks for a newer release of itself. Who must change: nobody. What users see: a top-level **Check for Updates…** item right after About Vestra Player… (order 881, group `windows`, no default key), an **Updates** row in About Vestra Player (current state, the latest version and its date, a Release Notes link and one action button: Check Now, Download, Cancel, Run Installer or Open Release Page; a newer version is drawn in the accent colour), and the settings page Options ▸ General ▸ Updates (automatic check, on by default).
- Source: `https://github.com/vestrasys/vestra-player/releases/latest/download/latest.json` (the repository is named once, in `updates.ts`). Schema 1 (`schema`, `version`, `published`, `notes`, `downloads.<platform>.{file,size,sha256}`); unknown fields are ignored, another schema major is refused ("needs a newer player"), versions are compared as semver with `plugin.app.version` (a player version that is not one is logged, and the window names the latest version without calling the player up to date). The platform key (`windows-x64`, `macos-arm64`) comes from `app.system()`. Redirects are followed by hand, https only.
- Automatic check: 30 s after the plugin starts, then at most once a day (`lastCheck` in `<data>/updates.json`, kept across runs); after a failed attempt the next one comes an hour later (`lastFailed`); a newer version shows one OSD notice per version (`notified`), never again for that version. Failures of the automatic check are only logged; Check for Updates… shows why (also when it joins an automatic check already running: the window says Checking… and then the result) (offline, 404 while the repository is private or has no release, 403/429 rate limits, other HTTP statuses, malformed JSON, a newer schema). Lazy activation: the timer does not keep the plugin running; when the player stops an idle Info plugin the pending check is dropped and the next start schedules it again from `lastCheck`.
- Download: the installer for this platform (only `.exe` on Windows, `.dmg` on macOS) comes from that release's own address, `releases/download/v<version>/<file>` (pinned like Tauri's updater and electron-updater, so a release published meanwhile cannot swap the file), and goes to the user's Downloads folder (`<file>.part`, then renamed; another file of that name is never overwritten, the installer becomes `<name> (1).<ext>`, `(2)`… like a browser's download), with progress; its size is counted while it streams and both size and sha256 must match `latest.json`, or the part file is deleted. A matching file already there is not downloaded again. A transfer that receives nothing for 30 seconds fails ("no data arrived for 30 seconds") and its part file is deleted. A check that ends while a download runs leaves the download, its state and its release alone. Run Installer checks the file again and opens it with the system (`system.openPath`: the setup program on Windows, the disk image on macOS); nothing is installed or replaced by the plugin. Without an installer for this platform the button opens the release page. While a download runs, Check for Updates… is busy ("Downloading the update…"), which keeps the plugin running until it ends.
- New capability `ui.settings` (the settings page). Network and opening files need none. Translations for all six languages (machine, marked `x-machine` for review).

## 0.1.10

- The About window's plugin list and the Playback Properties pages are Slint `ListView`s: only the rows in view are built, however many plugins or streams and tags a file has. Who must change: nobody. What users see: the same lists, rows, wrapping and scrolling; a value's text can still be selected and copied. One change: a selection in a value is lost when its row scrolls out of view and back (the list builds rows only while they are shown).
- Playback Properties no longer lays out every row's label (hidden) to size the label column. The plugin picks the page's widest label in the player's language by columns (wide CJK characters count two) and sends it as `label-sample`; the window lays out that one label and adds 8px, so a label about as long in a proportional font is not cut. The column is still at least 120px.
- The page tabs are the shared library `Tabs` (`@vestra/widgets.slint`) instead of a local copy: the same 74px strip, now with the keyboard (Left/Right/Home/End once focused with Tab) and the tab-list accessibility role. Clicking the tab already shown no longer clears the status line.
- Requires player 0.6.0 (`minPlayerVersion`), which ships the library `Tabs`.

## 0.1.9

- Lazy activation (player 0.5.21, SDK 1.5.40): the plugin no longer keeps running while idle. Who must change: nobody. What users see: nothing new; Playback Properties shows the same pages and follows the playing file while open. Under `VESTRA_LAZY=1` the player may now stop the plugin while nothing uses it and start it again on Tab, Ctrl+F1 or F1.
- The Playback Properties window's source listener (`player.onChange`) and its binding (the rows read `plugin.player.state`) are made in a child scope when the window opens and disposed when it closes (Close, the title bar, or a failed open, which now also stops the 1-second statistics polling). When it opens, the source is read again: a file opened while it was closed is shown at once, with fresh statistics. Before, both were made at activation and kept the plugin listening to every playback update. About's `packages.onChange` is unchanged: the player wakes the plugin for a package change.
- Two quick Ctrl+F1 (for example while the player starts the plugin) started two 1-second statistics polls, and closing the window stopped only one, so the other kept reading every second until the plugin stopped. The poll is now claimed before the first read, so an overlapping open starts none; the Close button stops polling before the window closes, so an open cannot land in between.
- Requires player 0.5.21 (`minPlayerVersion`).

## 0.1.8

- About Vestra Player shows how the video is rendered, below the component versions: **Rendering: GPU**, or **Rendering: Software — <reason>** with the player's own reason (for example that this player has no GPU renderer on Windows yet, or that the graphics driver is on the player's denylist, with where to choose another GPU in Windows' graphics settings). The reason is the player's English text, shown as it is; it wraps and the window grows to fit. Under GPU rendering a reason is shown only when it says more than "nothing rules GPU rendering out". Copy Info includes the same line.
- Read from `plugin.app.system()` (`rendering`, `renderingReason`) each time the window opens; no capability. Players before 0.5.12 report no renderer and the row is left out (not guessed); players before 0.5.13 report no reason and the row shows the mode alone. A failed read leaves the row out.
- Built with SDK 1.5.35 (the `renderingReason` type); `minPlayerVersion` stays 0.4.3.
