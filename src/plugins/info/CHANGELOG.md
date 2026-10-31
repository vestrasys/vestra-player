# Changelog

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
